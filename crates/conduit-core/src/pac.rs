//! PAC 引擎 —— 平移自 Python `server-app/core/pac_engine.py`，行为 100% 对齐。
//!
//! 解析项目自定义的 PAC 窄语法（5 段 numbered section + `shExpMatch` /
//! `dnsDomainIs` / `isInNet` 三个 helper），输出结构化决策。
//!
//! **不是**通用 JS PAC 解释器：浏览器仍然跑真正的 `proxy.pac` 文件，本模块
//! 只服务于 server 的 `/check?host=xxx` 诊断和 client 的智能路由决策。
//!
//! 5 段决策优先级：
//! 1. local / private / link-local        → DIRECT
//! 2. internal (must use VPN)             → PROXY (no DIRECT fallback)
//! 3. fallback (may need VPN)             → PROXY (no DIRECT fallback)
//! 4. CN direct                           → DIRECT
//! 5. default                             → DIRECT
//!
//! 设计参考：`design/2026-05-06-2-Conduit-Rust-重写设计文档.md` §4.2。

use std::net::Ipv4Addr;
use std::sync::OnceLock;

use globset::{Glob, GlobMatcher};
use ipnet::Ipv4Net;
use regex::Regex;

use crate::error::ConduitError;

const DEFAULT_PROXY_TARGET: &str = "PROXY 192.168.1.3:8080";
const DEFAULT_PROXY_TARGET_WITH_FALLBACK: &str = "PROXY 192.168.1.3:8080; DIRECT";

/// 已编译的 glob 规则；`pattern` 保留原 PAC 文件中的字面写法，
/// 用于 [`PacDecision::matched_pattern`] 输出。
#[derive(Debug, Clone)]
pub struct GlobRule {
    pub pattern: String,
    pub matcher: GlobMatcher,
}

impl GlobRule {
    fn compile(pattern: &str) -> Result<Self, ConduitError> {
        let glob = Glob::new(pattern)
            .map_err(|e| ConduitError::PacParse(format!("invalid glob '{pattern}': {e}")))?;
        Ok(Self {
            pattern: pattern.to_string(),
            matcher: glob.compile_matcher(),
        })
    }

    fn is_match(&self, host: &str) -> bool {
        self.matcher.is_match(host)
    }
}

/// `isInNet(host, ip, mask)` 解析结果；保留原 ip / mask 字符串用于
/// [`PacDecision::matched_pattern`] 输出。
#[derive(Debug, Clone)]
pub struct NetRule {
    pub ip: String,
    pub mask: String,
    pub net: Ipv4Net,
}

impl NetRule {
    fn compile(ip: &str, mask: &str) -> Option<Self> {
        let addr: Ipv4Addr = ip.parse().ok()?;
        let mask_addr: Ipv4Addr = mask.parse().ok()?;
        // Ipv4Net::with_netmask 会校验 netmask 是否是合法的 prefix（连续 1）
        let net = Ipv4Net::with_netmask(addr, mask_addr).ok()?;
        Some(Self {
            ip: ip.to_string(),
            mask: mask.to_string(),
            net,
        })
    }

    fn contains(&self, addr: Ipv4Addr) -> bool {
        self.net.contains(&addr)
    }
}

/// 单次决策结果，与 Python `Decision` 字段一一对应（snake_case 接口对齐）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacDecision {
    /// `"DIRECT"` 或 `"PROXY host:port"` 字面字符串，可直接喂给 PAC 调用方。
    pub proxy: String,
    /// 命中的 section 描述（如 `"1. local/private"`、`"5. default"`）。
    pub matched_section: &'static str,
    /// 命中的具体规则文本（如 `"*.local"`、`"dnsDomainIs(zoom.us)"`）。
    pub matched_pattern: String,
}

impl PacDecision {
    fn direct(section: &'static str, pattern: impl Into<String>) -> Self {
        Self {
            proxy: "DIRECT".to_string(),
            matched_section: section,
            matched_pattern: pattern.into(),
        }
    }

    fn proxy(target: &str, section: &'static str, pattern: impl Into<String>) -> Self {
        Self {
            proxy: target.to_string(),
            matched_section: section,
            matched_pattern: pattern.into(),
        }
    }
}

/// 解析后的 PAC 规则集合，与 Python `PacRules` 字段一一对应。
#[derive(Debug, Clone)]
pub struct PacRules {
    pub local_globs: Vec<GlobRule>,
    pub local_domains: Vec<String>,
    pub local_nets: Vec<NetRule>,
    pub internal_globs: Vec<GlobRule>,
    pub internal_domains: Vec<String>,
    pub fallback_globs: Vec<GlobRule>,
    pub fallback_domains: Vec<String>,
    pub cn_direct_globs: Vec<GlobRule>,
    pub cn_direct_domains: Vec<String>,
    pub proxy_target: String,
    pub proxy_target_with_fallback: String,
    pub source_path: String,
}

impl Default for PacRules {
    fn default() -> Self {
        Self {
            local_globs: Vec::new(),
            local_domains: Vec::new(),
            local_nets: Vec::new(),
            internal_globs: Vec::new(),
            internal_domains: Vec::new(),
            fallback_globs: Vec::new(),
            fallback_domains: Vec::new(),
            cn_direct_globs: Vec::new(),
            cn_direct_domains: Vec::new(),
            proxy_target: DEFAULT_PROXY_TARGET.to_string(),
            proxy_target_with_fallback: DEFAULT_PROXY_TARGET_WITH_FALLBACK.to_string(),
            source_path: String::new(),
        }
    }
}

// ---------- 正则表达式（与 Python 对齐） ----------

fn section_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?im)^\s*//\s*-{2,}\s*(\d+)\.\s*(.+?)\s*-{2,}\s*$")
            .expect("SECTION_RE compile")
    })
}

fn shexp_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"shExpMatch\s*\(\s*host\s*,\s*"([^"]+)"\s*\)"#).expect("SHEXP_RE compile")
    })
}

fn dnsdomain_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"dnsDomainIs\s*\(\s*host\s*,\s*"([^"]+)"\s*\)"#)
            .expect("DNSDOMAIN_RE compile")
    })
}

fn isinnet_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"isInNet\s*\(\s*host\s*,\s*"([\d.]+)"\s*,\s*"([\d.]+)"\s*\)"#)
            .expect("ISINNET_RE compile")
    })
}

// ---------- 内部 helper（与 Python `_xxx` 对齐，pub(crate) 暴露给单测） ----------

pub(crate) fn split_sections(text: &str) -> Vec<(u32, &str)> {
    let matches: Vec<_> = section_re().captures_iter(text).collect();
    if matches.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(matches.len());
    for (i, cap) in matches.iter().enumerate() {
        let idx: u32 = cap[1].parse().unwrap_or(0);
        let header_match = cap.get(0).expect("regex match always has group 0");
        let start = header_match.end();
        let end = matches
            .get(i + 1)
            .and_then(|nc| nc.get(0))
            .map(|m| m.start())
            .unwrap_or(text.len());
        out.push((idx, &text[start..end]));
    }
    out
}

pub(crate) fn is_plain_host(host: &str) -> bool {
    !host.contains('.')
}

pub(crate) fn looks_like_ipv4(host: &str) -> bool {
    host.parse::<Ipv4Addr>().is_ok()
}

/// 与 Python `_ip_in_net` 行为一致；main 代码使用 [`NetRule::contains`] 直接判定，
/// 这个 helper 仅保留给单测对齐 Python helper 级用例。
#[cfg(test)]
pub(crate) fn ip_in_net(ip_str: &str, net_ip: &str, net_mask: &str) -> bool {
    let Ok(addr) = ip_str.parse::<Ipv4Addr>() else {
        return false;
    };
    let Some(rule) = NetRule::compile(net_ip, net_mask) else {
        return false;
    };
    rule.contains(addr)
}

pub(crate) fn glob_any<'a>(host: &str, globs: &'a [GlobRule]) -> Option<&'a str> {
    globs
        .iter()
        .find(|g| g.is_match(host))
        .map(|g| g.pattern.as_str())
}

pub(crate) fn domain_any<'a>(host: &str, domains: &'a [String]) -> Option<&'a str> {
    for d in domains {
        let d_norm = d.trim_start_matches('.').to_lowercase();
        if host == d_norm || host.ends_with(&format!(".{d_norm}")) {
            return Some(d.as_str());
        }
    }
    None
}

// ---------- 主 API ----------

impl PacRules {
    /// 从已读入内存的 PAC 文本解析 5 段规则。
    ///
    /// 与 Python `load_rules` 不同的是：本方法不做文件 IO，调用方负责读文件。
    /// 这样允许 `include_str!("../../proxy.pac")` 在编译期嵌入资源，避免
    /// runtime 路径查找。
    ///
    /// 任何 glob 编译失败会被记录但不阻断（与 Python `fnmatch` 容错语义一致 —
    /// Python 端 fnmatch 不会预编译，只在 match 时才报错）。这里我们
    /// 静默丢弃非法 glob，保证 runtime 不崩。
    pub fn parse(text: &str) -> Self {
        let mut rules = Self::default();
        let sections = split_sections(text);
        for (idx, body) in sections {
            let globs = collect_globs(body);
            let domains = collect_domains(body);
            let nets = collect_nets(body);
            match idx {
                1 => {
                    rules.local_globs = globs;
                    rules.local_domains = domains;
                    rules.local_nets = nets;
                }
                2 => {
                    rules.internal_globs = globs;
                    rules.internal_domains = domains;
                }
                3 => {
                    rules.fallback_globs = globs;
                    rules.fallback_domains = domains;
                }
                4 => {
                    rules.cn_direct_globs = globs;
                    rules.cn_direct_domains = domains;
                }
                _ => {
                    // section 5 是默认分支，无需提取规则
                }
            }
        }
        rules
    }

    /// 更新 PROXY 字面值（替换 PAC 文件里的 `__PROXY_HOST__` / `__PROXY_PORT__` 占位）。
    pub fn update_proxy_target(&mut self, host: &str, http_port: u16) {
        self.proxy_target = format!("PROXY {host}:{http_port}");
        self.proxy_target_with_fallback = format!("PROXY {host}:{http_port}; DIRECT");
    }

    /// 决策一个 host 应该走 DIRECT 还是 PROXY，输出含命中段与命中模式。
    /// 行为与 Python `find_proxy(host, rules)` 100% 对齐。
    pub fn find_proxy(&self, host: &str) -> PacDecision {
        let host = host.trim().to_lowercase();
        if host.is_empty() {
            return PacDecision::direct("5. default", "(empty host)");
        }

        // ---------- 1. local / private ----------
        if is_plain_host(&host) || host == "localhost" {
            return PacDecision::direct("1. local/private", "isPlainHostName / localhost");
        }
        if let Some(g) = glob_any(&host, &self.local_globs) {
            return PacDecision::direct("1. local/private", g);
        }
        if let Some(d) = domain_any(&host, &self.local_domains) {
            return PacDecision::direct("1. local/private", format!("dnsDomainIs({d})"));
        }
        if looks_like_ipv4(&host) {
            if let Ok(addr) = host.parse::<Ipv4Addr>() {
                for net in &self.local_nets {
                    if net.contains(addr) {
                        return PacDecision::direct(
                            "1. local/private",
                            format!("isInNet({}/{})", net.ip, net.mask),
                        );
                    }
                }
            }
        }

        // ---------- 2. internal (must use VPN) ----------
        if let Some(g) = glob_any(&host, &self.internal_globs) {
            return PacDecision::proxy(&self.proxy_target, "2. internal (must use VPN)", g);
        }
        if let Some(d) = domain_any(&host, &self.internal_domains) {
            return PacDecision::proxy(
                &self.proxy_target,
                "2. internal (must use VPN)",
                format!("dnsDomainIs({d})"),
            );
        }

        // ---------- 3. fallback (may need VPN) ----------
        if let Some(g) = glob_any(&host, &self.fallback_globs) {
            return PacDecision::proxy(
                &self.proxy_target,
                "3. may need VPN (proxy, no DIRECT fallback)",
                g,
            );
        }
        if let Some(d) = domain_any(&host, &self.fallback_domains) {
            return PacDecision::proxy(
                &self.proxy_target,
                "3. may need VPN (proxy, no DIRECT fallback)",
                format!("dnsDomainIs({d})"),
            );
        }

        // ---------- 4. CN direct ----------
        if let Some(g) = glob_any(&host, &self.cn_direct_globs) {
            return PacDecision::direct("4. CN direct", g);
        }
        if let Some(d) = domain_any(&host, &self.cn_direct_domains) {
            return PacDecision::direct("4. CN direct", format!("dnsDomainIs({d})"));
        }

        // ---------- 5. default ----------
        PacDecision::direct("5. default", "(no rule matched)")
    }
}

fn collect_globs(body: &str) -> Vec<GlobRule> {
    shexp_re()
        .captures_iter(body)
        .filter_map(|cap| GlobRule::compile(&cap[1]).ok())
        .collect()
}

fn collect_domains(body: &str) -> Vec<String> {
    dnsdomain_re()
        .captures_iter(body)
        .map(|cap| cap[1].to_string())
        .collect()
}

fn collect_nets(body: &str) -> Vec<NetRule> {
    isinnet_re()
        .captures_iter(body)
        .filter_map(|cap| NetRule::compile(&cap[1], &cap[2]))
        .collect()
}

#[cfg(test)]
mod tests {
    //! 全部测试 case 一一对齐 `server-app/core/tests/test_pac_engine.py`，
    //! 任何行为差异都视为回归。

    use super::*;

    fn make_globs(patterns: &[&str]) -> Vec<GlobRule> {
        patterns
            .iter()
            .filter_map(|p| GlobRule::compile(p).ok())
            .collect()
    }

    // ---------- 工具函数 ----------

    #[test]
    fn is_plain_host_recognises_bare_names() {
        assert!(is_plain_host("intranet"));
        assert!(is_plain_host("dev-box"));
        assert!(!is_plain_host("example.com"));
        assert!(!is_plain_host("a.b"));
    }

    #[test]
    fn looks_like_ipv4_only_accepts_real_ipv4() {
        assert!(looks_like_ipv4("192.168.1.1"));
        assert!(looks_like_ipv4("10.0.0.0"));
        assert!(!looks_like_ipv4("example.com"));
        assert!(!looks_like_ipv4("10.0.0.300"));
    }

    #[test]
    fn ip_in_net_handles_cidr_correctly() {
        assert!(ip_in_net("192.168.1.5", "192.168.0.0", "255.255.0.0"));
        assert!(ip_in_net("10.1.2.3", "10.0.0.0", "255.0.0.0"));
        assert!(!ip_in_net("172.16.0.1", "192.168.0.0", "255.255.0.0"));
    }

    #[test]
    fn ip_in_net_invalid_input_returns_false() {
        assert!(!ip_in_net("not-an-ip", "192.168.0.0", "255.255.0.0"));
        assert!(!ip_in_net("192.168.1.1", "bogus", "255.255.0.0"));
    }

    #[test]
    fn glob_any_matches_first_pattern() {
        let globs = make_globs(&["bar.*", "*.example.com"]);
        assert_eq!(glob_any("foo.example.com", &globs), Some("*.example.com"));
        let globs2 = make_globs(&["*.example.com"]);
        assert_eq!(glob_any("baz.com", &globs2), None);
    }

    #[test]
    fn domain_any_matches_dnsdomainis_semantics() {
        let domains = vec!["example.com".to_string()];
        assert_eq!(domain_any("foo.example.com", &domains), Some("example.com"));
        assert_eq!(domain_any("example.com", &domains), Some("example.com"));
        assert_eq!(domain_any("notexample.com", &domains), None);

        let leading_dot = vec![".example.com".to_string()];
        assert_eq!(
            domain_any("example.com", &leading_dot),
            Some(".example.com")
        );
    }

    // ---------- _split_sections ----------

    #[test]
    fn split_sections_extracts_each_block_in_order() {
        let text = r#"
        // some preamble
        // ---------- 1. local ----------
        rule_a
        // ---------- 2. internal ----------
        rule_b
        // ---------- 3. fallback ----------
        rule_c
    "#;
        let sections = split_sections(text);
        let keys: Vec<u32> = sections.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec![1, 2, 3]);
        let map: std::collections::HashMap<u32, &str> = sections.into_iter().collect();
        assert!(map[&1].contains("rule_a"));
        assert!(map[&2].contains("rule_b"));
        assert!(map[&3].contains("rule_c"));
    }

    #[test]
    fn split_sections_empty_returns_empty_vec() {
        assert!(split_sections("// nothing here").is_empty());
    }

    // ---------- PacRules::parse / update_proxy_target ----------

    const SAMPLE_PAC: &str = r#"
        // ---------- 1. local/private ----------
        if (shExpMatch(host, "*.local")) return "DIRECT";
        if (dnsDomainIs(host, "lan.example")) return "DIRECT";
        if (isInNet(host, "10.0.0.0", "255.0.0.0")) return "DIRECT";

        // ---------- 2. internal (must use VPN) ----------
        if (shExpMatch(host, "*.corp.example")) return "PROXY 1.2.3.4:8080";
        if (dnsDomainIs(host, "internal.example")) return "PROXY 1.2.3.4:8080";

        // ---------- 3. fallback (proxy w/o DIRECT) ----------
        if (shExpMatch(host, "*.cdn.example")) return "PROXY 1.2.3.4:8080";

        // ---------- 4. CN direct ----------
        if (shExpMatch(host, "*.cn")) return "DIRECT";
        if (dnsDomainIs(host, "baidu.com")) return "DIRECT";
    "#;

    #[test]
    fn parse_extracts_all_section_helpers() {
        let rules = PacRules::parse(SAMPLE_PAC);
        let local_glob_patterns: Vec<_> =
            rules.local_globs.iter().map(|g| g.pattern.as_str()).collect();
        assert_eq!(local_glob_patterns, vec!["*.local"]);
        assert_eq!(rules.local_domains, vec!["lan.example".to_string()]);

        assert_eq!(rules.local_nets.len(), 1);
        assert_eq!(rules.local_nets[0].ip, "10.0.0.0");
        assert_eq!(rules.local_nets[0].mask, "255.0.0.0");

        let internal_glob_patterns: Vec<_> = rules
            .internal_globs
            .iter()
            .map(|g| g.pattern.as_str())
            .collect();
        assert_eq!(internal_glob_patterns, vec!["*.corp.example"]);
        assert_eq!(rules.internal_domains, vec!["internal.example".to_string()]);

        let fallback_glob_patterns: Vec<_> = rules
            .fallback_globs
            .iter()
            .map(|g| g.pattern.as_str())
            .collect();
        assert_eq!(fallback_glob_patterns, vec!["*.cdn.example"]);

        let cn_glob_patterns: Vec<_> = rules
            .cn_direct_globs
            .iter()
            .map(|g| g.pattern.as_str())
            .collect();
        assert_eq!(cn_glob_patterns, vec!["*.cn"]);
        assert_eq!(rules.cn_direct_domains, vec!["baidu.com".to_string()]);
    }

    #[test]
    fn parse_empty_text_returns_default_rules() {
        let rules = PacRules::parse("");
        assert!(rules.local_globs.is_empty());
        assert!(rules.internal_globs.is_empty());
        assert!(rules.proxy_target.starts_with("PROXY "));
    }

    #[test]
    fn update_proxy_target_updates_both_strings() {
        let mut rules = PacRules::default();
        rules.update_proxy_target("server.local", 9999);
        assert_eq!(rules.proxy_target, "PROXY server.local:9999");
        assert_eq!(
            rules.proxy_target_with_fallback,
            "PROXY server.local:9999; DIRECT"
        );
    }

    // ---------- find_proxy 语义 ----------

    fn loaded_rules() -> PacRules {
        let mut rules = PacRules::parse(SAMPLE_PAC);
        rules.update_proxy_target("up", 7777);
        rules
    }

    #[test]
    fn find_proxy_empty_host_returns_default_direct() {
        let r = loaded_rules();
        let d = r.find_proxy("");
        assert_eq!(d.proxy, "DIRECT");
        assert!(d.matched_section.starts_with("5."));
    }

    #[test]
    fn find_proxy_plain_host_takes_section_1() {
        let r = loaded_rules();
        let d = r.find_proxy("intranet");
        assert_eq!(d.proxy, "DIRECT");
        assert!(d.matched_section.contains("local"));
    }

    #[test]
    fn find_proxy_localhost_special_case() {
        let r = loaded_rules();
        let d = r.find_proxy("localhost");
        assert_eq!(d.proxy, "DIRECT");
    }

    #[test]
    fn find_proxy_section1_glob_match() {
        let r = loaded_rules();
        let d = r.find_proxy("foo.local");
        assert_eq!(d.proxy, "DIRECT");
        assert_eq!(d.matched_pattern, "*.local");
    }

    #[test]
    fn find_proxy_section1_dnsdomain_match() {
        let r = loaded_rules();
        let d = r.find_proxy("box.lan.example");
        assert_eq!(d.proxy, "DIRECT");
        assert_eq!(d.matched_pattern, "dnsDomainIs(lan.example)");
    }

    #[test]
    fn find_proxy_section1_isinnet_match() {
        let r = loaded_rules();
        let d = r.find_proxy("10.1.2.3");
        assert_eq!(d.proxy, "DIRECT");
        assert!(d.matched_pattern.contains("isInNet"));
    }

    #[test]
    fn find_proxy_section2_internal_uses_proxy_target() {
        let r = loaded_rules();
        let d = r.find_proxy("svc.corp.example");
        assert_eq!(d.proxy, "PROXY up:7777");
        assert!(d.matched_section.starts_with("2."));
    }

    #[test]
    fn find_proxy_section3_fallback() {
        let r = loaded_rules();
        let d = r.find_proxy("img.cdn.example");
        assert_eq!(d.proxy, "PROXY up:7777");
        assert!(d.matched_section.starts_with("3."));
    }

    #[test]
    fn find_proxy_section4_cn_direct() {
        let r = loaded_rules();
        let d = r.find_proxy("anything.cn");
        assert_eq!(d.proxy, "DIRECT");
        assert!(d.matched_section.starts_with("4."));
    }

    #[test]
    fn find_proxy_default_falls_through() {
        let r = loaded_rules();
        let d = r.find_proxy("unmatched.example.org");
        assert_eq!(d.proxy, "DIRECT");
        assert!(d.matched_section.starts_with("5."));
        assert!(d.matched_pattern.contains("no rule matched"));
    }

    #[test]
    fn find_proxy_case_insensitive_host() {
        let r = loaded_rules();
        let d = r.find_proxy("FOO.LOCAL");
        assert_eq!(d.proxy, "DIRECT");
        assert_eq!(d.matched_pattern, "*.local");
    }

    // ---------- 真实 proxy.pac 决策 ----------
    //
    // 嵌入项目的真实 PAC 文件做一轮决策对齐，避免合成测试遗漏真实场景。

    const REAL_PAC: &str = include_str!("../../../server-app/core/proxy.pac");

    fn real_rules() -> PacRules {
        let mut r = PacRules::parse(REAL_PAC);
        r.update_proxy_target("conduit-server.local", 8080);
        r
    }

    #[test]
    fn real_pac_loads_all_five_sections() {
        let r = PacRules::parse(REAL_PAC);
        // 真实 PAC section 1 含 `shExpMatch(host, "localhost")` + 多条
        // dnsDomainIs (local/lan/internal) + 5 条 isInNet
        let local_glob_patterns: Vec<&str> =
            r.local_globs.iter().map(|g| g.pattern.as_str()).collect();
        assert_eq!(local_glob_patterns, vec!["localhost"]);
        assert!(!r.local_domains.is_empty());
        assert_eq!(r.local_nets.len(), 5);
        // section 2 / 3 / 4 都纯靠 dnsDomainIs，无 globs
        assert!(r.internal_globs.is_empty());
        assert!(!r.internal_domains.is_empty());
        assert!(r.fallback_globs.is_empty());
        assert!(!r.fallback_domains.is_empty());
        assert!(r.cn_direct_globs.is_empty());
        assert!(!r.cn_direct_domains.is_empty());
    }

    #[test]
    fn real_pac_internal_zoom_routes_via_proxy() {
        let r = real_rules();
        let d = r.find_proxy("git.zoom.us");
        assert_eq!(d.proxy, "PROXY conduit-server.local:8080");
        assert!(d.matched_section.starts_with("2."));
    }

    #[test]
    fn real_pac_fallback_google_routes_via_proxy() {
        let r = real_rules();
        let d = r.find_proxy("www.google.com");
        assert_eq!(d.proxy, "PROXY conduit-server.local:8080");
        assert!(d.matched_section.starts_with("3."));
    }

    #[test]
    fn real_pac_cn_direct_baidu_routes_direct() {
        let r = real_rules();
        let d = r.find_proxy("baidu.com");
        assert_eq!(d.proxy, "DIRECT");
        assert!(d.matched_section.starts_with("4."));
    }

    #[test]
    fn real_pac_local_lan_routes_direct() {
        let r = real_rules();
        let d = r.find_proxy("192.168.1.10");
        assert_eq!(d.proxy, "DIRECT");
        assert!(d.matched_section.starts_with("1."));
    }

    #[test]
    fn real_pac_unknown_host_falls_through_to_direct() {
        let r = real_rules();
        let d = r.find_proxy("random-blog.io");
        assert_eq!(d.proxy, "DIRECT");
        assert!(d.matched_section.starts_with("5."));
    }
}

