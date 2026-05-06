//! `RouteResolver` —— 把 `host:port` 决策为 `direct` 或 `proxy`。
//!
//! 决策树：
//!
//! 1. **Global override** —— 如果 `set_global_mode("a_unreachable")`，全部 direct。
//! 2. **Private-IP fast path** —— RFC1918 / loopback / link-local 全部 direct。
//! 3. **Cache lookup** —— 命中 host 直接返。
//! 4. **TCP probe** —— 1.5s connect 测试（直连），失败 → proxy。
//! 5. （Self-heal 由 `LocalProxy` 调 `cache.flip` 完成，不在 resolver 内）。

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use log::debug;
use tokio::net::TcpStream;
use tokio::time::timeout;

use conduit_core::{RouteDirection, RouteEntry};

use super::route_cache::{RouteCache, DEFAULT_PROBE_TTL_SEC};

/// 单次 TCP probe 超时。
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// 一次决策的来源标识（用于 UI 展示「为什么走这条路」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionSource {
    GlobalOverride,
    PrivateIp,
    Cache,
    Probe,
}

impl DecisionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GlobalOverride => "global_override",
            Self::PrivateIp => "private_ip",
            Self::Cache => "cache",
            Self::Probe => "probe",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub direction: RouteDirection,
    pub source: DecisionSource,
    pub cache_entry: Option<RouteEntry>,
}

#[derive(Clone)]
pub struct RouteResolver {
    cache: RouteCache,
    /// 当 server 不可达时由 connectivity 设置为 true，全部走 direct。
    global_a_unreachable: Arc<AtomicBool>,
}

impl RouteResolver {
    pub fn new(cache: RouteCache) -> Self {
        Self {
            cache,
            global_a_unreachable: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cache(&self) -> RouteCache {
        self.cache.clone()
    }

    pub fn set_global_mode(&self, a_unreachable: bool) {
        self.global_a_unreachable
            .store(a_unreachable, Ordering::Release);
    }

    pub fn is_globally_degraded(&self) -> bool {
        self.global_a_unreachable.load(Ordering::Acquire)
    }

    /// 主决策入口。
    ///
    /// `port` 仅用于第 4 步 probe；前几步是按 host 决策。
    pub async fn decide(&self, host: &str, port: u16) -> RouteDecision {
        if self.global_a_unreachable.load(Ordering::Acquire) {
            return RouteDecision {
                direction: RouteDirection::Direct,
                source: DecisionSource::GlobalOverride,
                cache_entry: None,
            };
        }
        if is_private_or_loopback(host) {
            return RouteDecision {
                direction: RouteDirection::Direct,
                source: DecisionSource::PrivateIp,
                cache_entry: None,
            };
        }
        if let Some(entry) = self.cache.get(host) {
            let direction = entry.direction;
            return RouteDecision {
                direction,
                source: DecisionSource::Cache,
                cache_entry: Some(entry),
            };
        }
        let direction = if probe_direct(host, port).await {
            RouteDirection::Direct
        } else {
            RouteDirection::Proxy
        };
        // memoize 决策
        self.cache.set_with_ttl(host, direction, "probe", DEFAULT_PROBE_TTL_SEC);
        debug!("[resolver] probe {host}:{port} → {direction:?}");
        let entry = self.cache.get(host);
        RouteDecision {
            direction,
            source: DecisionSource::Probe,
            cache_entry: entry,
        }
    }
}

/// 判断字符串是否为 RFC1918 / loopback / link-local IPv4/IPv6。
///
/// 域名一律返 false（不做 DNS 解析，只看裸 IP 字面量）。
pub fn is_private_or_loopback(host: &str) -> bool {
    let Ok(ip) = host.parse::<IpAddr>() else {
        return false;
    };
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

async fn probe_direct(host: &str, port: u16) -> bool {
    let target = format!("{host}:{port}");
    matches!(
        timeout(PROBE_TIMEOUT, TcpStream::connect(&target)).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_classification_covers_rfc1918_loopback_linklocal() {
        assert!(is_private_or_loopback("127.0.0.1"));
        assert!(is_private_or_loopback("10.0.0.5"));
        assert!(is_private_or_loopback("192.168.1.1"));
        assert!(is_private_or_loopback("172.16.0.1"));
        assert!(is_private_or_loopback("169.254.1.1"));
        assert!(!is_private_or_loopback("8.8.8.8"));
        assert!(!is_private_or_loopback("example.com"));
    }

    #[tokio::test]
    async fn global_override_short_circuits_to_direct() {
        let r = RouteResolver::new(RouteCache::new());
        r.set_global_mode(true);
        let d = r.decide("google.com", 443).await;
        assert_eq!(d.direction, RouteDirection::Direct);
        assert_eq!(d.source, DecisionSource::GlobalOverride);
    }

    #[tokio::test]
    async fn private_ip_returns_direct_without_probe() {
        let r = RouteResolver::new(RouteCache::new());
        let d = r.decide("192.168.1.5", 22).await;
        assert_eq!(d.direction, RouteDirection::Direct);
        assert_eq!(d.source, DecisionSource::PrivateIp);
    }

    #[tokio::test]
    async fn cache_hit_skips_probe() {
        let cache = RouteCache::new();
        cache.set_with_ttl("example.com", RouteDirection::Proxy, "pac", 60.0);
        let r = RouteResolver::new(cache);
        let d = r.decide("example.com", 443).await;
        assert_eq!(d.direction, RouteDirection::Proxy);
        assert_eq!(d.source, DecisionSource::Cache);
    }
}
