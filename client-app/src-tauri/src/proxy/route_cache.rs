//! `RouteCache` —— 路由决策缓存（host → direct/proxy 决策）。
//!
//! 设计：
//! - 后端用 `dashmap` 做并发哈希表（key = host 小写）。
//! - 每条 entry 带 `expires_at` epoch 秒；过期不立即删除，下次 `get` 时返回 None 并清理。
//! - 容量 4096，超过时按 last-touch 时间淘汰最旧条目（lazy 在 set 时检查）。
//! - PAC prefill 用 `set_with_ttl(host, direction, "pac", 5min)`。
//!
//! 不上 `moka` 的原因：moka 引入巨量依赖（编译变慢 30s+），而我们需要的语义
//! 极简（host → direction + ttl），dashmap + 自己写过期逻辑足够。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;

use conduit_core::{RouteDirection, RouteEntry};

/// 默认 PAC prefill TTL：5 分钟内的 PAC 命中不再回源决策。
pub const DEFAULT_PREFILL_TTL_SEC: f64 = 300.0;

/// 默认 probe TTL：10 分钟内的同 host probe 结果可复用。
pub const DEFAULT_PROBE_TTL_SEC: f64 = 600.0;

/// 容量上限（hashmap 长度）。
pub const MAX_ENTRIES: usize = 4096;

#[derive(Clone)]
pub struct RouteCache {
    inner: Arc<DashMap<String, RouteEntry>>,
    /// 单调递增 hit 计数，用于命中事件 payload。
    total_hits: Arc<AtomicU64>,
}

impl RouteCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::with_capacity(256)),
            total_hits: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 根据 host 查询缓存（host 自动小写化）。过期返 None 并删除。
    pub fn get(&self, host: &str) -> Option<RouteEntry> {
        let key = host.to_lowercase();
        let now = epoch_now();
        if let Some(mut entry) = self.inner.get_mut(&key) {
            if entry.expires_at < now {
                drop(entry);
                self.inner.remove(&key);
                return None;
            }
            entry.hit_count += 1;
            self.total_hits.fetch_add(1, Ordering::Relaxed);
            return Some(entry.clone());
        }
        None
    }

    /// 写入或覆盖 entry。
    pub fn set(&self, host: String, mut entry: RouteEntry) {
        entry.host = host.to_lowercase();
        let key = entry.host.clone();
        if self.inner.len() >= MAX_ENTRIES {
            self.evict_oldest();
        }
        self.inner.insert(key, entry);
    }

    /// 便捷构造：`(host, direction, source, ttl_sec)`。
    pub fn set_with_ttl(
        &self,
        host: &str,
        direction: RouteDirection,
        source: &str,
        ttl_sec: f64,
    ) {
        let entry = RouteEntry {
            host: host.to_lowercase(),
            direction,
            expires_at: epoch_now() + ttl_sec,
            source: source.to_string(),
            hit_count: 0,
        };
        self.set(host.to_string(), entry);
    }

    /// 当前缓存条目数（含未过期和已过期但还没 lazy 清理的）。
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// 全量快照，按 host 字典序，用于 `/api/cache` REST。
    pub fn snapshot(&self) -> Vec<RouteEntry> {
        let mut out: Vec<RouteEntry> = self.inner.iter().map(|kv| kv.value().clone()).collect();
        out.sort_by(|a, b| a.host.cmp(&b.host));
        out
    }

    /// 清空全部（用于 `/api/cache DELETE`）。
    pub fn clear(&self) -> usize {
        let n = self.inner.len();
        self.inner.clear();
        n
    }

    /// 把单条 entry 标记为命中失败（self-heal），翻转 direction 并续期。
    pub fn flip(&self, host: &str) -> Option<RouteEntry> {
        let key = host.to_lowercase();
        let mut entry_ref = self.inner.get_mut(&key)?;
        entry_ref.direction = match entry_ref.direction {
            RouteDirection::Direct => RouteDirection::Proxy,
            RouteDirection::Proxy => RouteDirection::Direct,
        };
        entry_ref.expires_at = epoch_now() + DEFAULT_PROBE_TTL_SEC;
        entry_ref.source = "self_heal".to_string();
        Some(entry_ref.clone())
    }

    fn evict_oldest(&self) {
        // 找 expires_at 最小（最快过期 / 已过期）的条目删除。
        let mut victim: Option<(String, f64)> = None;
        for kv in self.inner.iter() {
            let exp = kv.value().expires_at;
            match &victim {
                None => victim = Some((kv.key().clone(), exp)),
                Some((_, cur)) if exp < *cur => victim = Some((kv.key().clone(), exp)),
                _ => {}
            }
        }
        if let Some((key, _)) = victim {
            self.inner.remove(&key);
        }
    }
}

impl Default for RouteCache {
    fn default() -> Self {
        Self::new()
    }
}

fn epoch_now() -> f64 {
    Utc::now().timestamp_micros() as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(host: &str, dir: RouteDirection, ttl: f64) -> RouteEntry {
        RouteEntry {
            host: host.into(),
            direction: dir,
            expires_at: epoch_now() + ttl,
            source: "test".into(),
            hit_count: 0,
        }
    }

    #[test]
    fn get_returns_none_for_missing() {
        let c = RouteCache::new();
        assert!(c.get("example.com").is_none());
    }

    #[test]
    fn set_then_get_roundtrip_lowercases_host() {
        let c = RouteCache::new();
        c.set("Example.COM".into(), entry("placeholder", RouteDirection::Proxy, 60.0));
        let got = c.get("example.com").unwrap();
        assert_eq!(got.direction, RouteDirection::Proxy);
        assert_eq!(got.host, "example.com");
        assert_eq!(got.hit_count, 1);
    }

    #[test]
    fn expired_entry_is_evicted_on_get() {
        let c = RouteCache::new();
        c.set("ex.com".into(), entry("ex.com", RouteDirection::Direct, -1.0));
        assert!(c.get("ex.com").is_none());
        assert_eq!(c.len(), 0, "expired entry should have been removed");
    }

    #[test]
    fn flip_inverts_direction_and_resets_ttl() {
        let c = RouteCache::new();
        c.set_with_ttl("ex.com", RouteDirection::Direct, "probe", 10.0);
        let flipped = c.flip("ex.com").unwrap();
        assert_eq!(flipped.direction, RouteDirection::Proxy);
        assert_eq!(flipped.source, "self_heal");
    }

    #[test]
    fn clear_resets_all_entries() {
        let c = RouteCache::new();
        for i in 0..5 {
            c.set_with_ttl(&format!("h{i}.com"), RouteDirection::Proxy, "pac", 60.0);
        }
        assert_eq!(c.clear(), 5);
        assert!(c.is_empty());
    }

    #[test]
    fn snapshot_sorted_by_host() {
        let c = RouteCache::new();
        c.set_with_ttl("c.com", RouteDirection::Proxy, "pac", 60.0);
        c.set_with_ttl("a.com", RouteDirection::Direct, "probe", 60.0);
        c.set_with_ttl("b.com", RouteDirection::Proxy, "manual", 60.0);
        let snap = c.snapshot();
        assert_eq!(snap[0].host, "a.com");
        assert_eq!(snap[1].host, "b.com");
        assert_eq!(snap[2].host, "c.com");
    }
}
