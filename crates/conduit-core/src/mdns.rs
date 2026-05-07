//! mDNS / Bonjour 服务广播契约 —— Server 端广播 / Client 端发现共享。
//!
//! 所有字段名是双端契约，server 写、client 读，调整时必须双端同步：
//! - server-app `proxy::mdns` 调 [`build_txt`] 写 TXT 字段
//! - client-app `proxy::discoverer` 调 [`parse_txt`] 读 TXT 字段
//!
//! 任何 TXT 字段调整必须同时修改 [`build_txt`] 与 [`parse_txt`]，并跑双端兼容回归测试。
//!
//! 实际网络层（绑定 / 浏览 / 监听 ServiceStateChange）由 server-app / client-app
//! 自己的 mdns 模块用 [`mdns-sd`](https://crates.io/crates/mdns-sd) 实现，本模块只负责
//! **协议常量** + **类型 + TXT 编解码**。

use std::collections::HashMap;

/// mDNS service type。
pub const SERVICE_TYPE: &str = "_conduit._tcp.local.";

/// PAC URL 默认相对路径，TXT 中 `pac` 字段缺失时的回退值。
pub const DEFAULT_PAC_PATH: &str = "/proxy.pac";

/// TXT 记录字段名常量，集中定义避免 typo。
pub mod txt {
    pub const NAME: &str = "name";
    pub const PORT: &str = "port";
    pub const SOCKS: &str = "socks";
    pub const API: &str = "api";
    pub const VPN: &str = "vpn";
    pub const VERSION: &str = "version";
    pub const PAC: &str = "pac";
}

/// 单条 mDNS 广播的负载。Server 用其调用 [`build_txt`] 生成 TXT 字典，
/// Client 用 [`parse_txt`] 把扫到的 TXT 转换回 [`MdnsServiceInfo`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsServiceInfo {
    pub name: String,
    pub http_port: u16,
    pub socks_port: u16,
    pub api_port: u16,
    pub vpn_on: bool,
    pub version: String,
    pub pac_path: String,
}

impl MdnsServiceInfo {
    /// 渲染 mDNS instance 名：`Conduit on {name}._conduit._tcp.local.`。
    pub fn instance_fqdn(&self) -> String {
        format!("Conduit on {}.{}", self.name, SERVICE_TYPE)
    }

    /// 用作 SRV `server` 字段的主机名：`{name}.local.`。
    pub fn server_fqdn(&self) -> String {
        format!("{}.local.", self.name)
    }
}

/// 把 [`MdnsServiceInfo`] 序列化为 TXT 字典（key/value 都是 String）。
///
/// `vpn` 字段：开 → `"on"`，关 → `"off"`（client 端按字符串解析）。
pub fn build_txt(info: &MdnsServiceInfo) -> HashMap<String, String> {
    let mut out = HashMap::with_capacity(7);
    out.insert(txt::NAME.into(), info.name.clone());
    out.insert(txt::PORT.into(), info.http_port.to_string());
    out.insert(txt::SOCKS.into(), info.socks_port.to_string());
    out.insert(txt::API.into(), info.api_port.to_string());
    out.insert(
        txt::VPN.into(),
        if info.vpn_on { "on" } else { "off" }.into(),
    );
    out.insert(txt::VERSION.into(), info.version.clone());
    out.insert(txt::PAC.into(), info.pac_path.clone());
    out
}

/// 反序列化错误。所有数值字段不可解析时回退或报错（取决于字段是否必需）。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MdnsParseError {
    #[error("missing required TXT field `{0}`")]
    MissingField(&'static str),
    #[error("invalid TXT field `{field}`: {reason}")]
    InvalidField {
        field: &'static str,
        reason: String,
    },
}

/// 从 TXT 字典反序列化。`fallback_http_port` 用作 `port` 字段缺失时的回退。
pub fn parse_txt(
    txt: &HashMap<String, String>,
    fallback_http_port: u16,
) -> Result<MdnsServiceInfo, MdnsParseError> {
    let parse_port = |key: &'static str, default: Option<u16>| -> Result<u16, MdnsParseError> {
        match txt.get(key) {
            Some(v) => v.parse::<u16>().map_err(|e| MdnsParseError::InvalidField {
                field: key,
                reason: e.to_string(),
            }),
            None => default.ok_or(MdnsParseError::MissingField(key)),
        }
    };
    let http_port = parse_port(txt::PORT, Some(fallback_http_port))?;
    let socks_port = parse_port(txt::SOCKS, Some(0))?;
    let api_port = parse_port(txt::API, Some(0))?;
    let name = txt
        .get(txt::NAME)
        .cloned()
        .ok_or(MdnsParseError::MissingField(txt::NAME))?;
    let vpn_on = matches!(txt.get(txt::VPN).map(String::as_str), Some("on"));
    let version = txt.get(txt::VERSION).cloned().unwrap_or_default();
    let pac_path = txt
        .get(txt::PAC)
        .cloned()
        .unwrap_or_else(|| DEFAULT_PAC_PATH.into());
    Ok(MdnsServiceInfo {
        name,
        http_port,
        socks_port,
        api_port,
        vpn_on,
        version,
        pac_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MdnsServiceInfo {
        MdnsServiceInfo {
            name: "host01".into(),
            http_port: 8080,
            socks_port: 1080,
            api_port: 8090,
            vpn_on: true,
            version: "0.1.4".into(),
            pac_path: "/proxy.pac".into(),
        }
    }

    #[test]
    fn build_then_parse_roundtrips() {
        let info = sample();
        let txt = build_txt(&info);
        assert_eq!(txt.get("vpn").map(|s| s.as_str()), Some("on"));
        assert_eq!(txt.get("port").map(|s| s.as_str()), Some("8080"));
        let parsed = parse_txt(&txt, 0).unwrap();
        assert_eq!(parsed, info);
    }

    #[test]
    fn build_uses_off_for_disabled_vpn() {
        let mut info = sample();
        info.vpn_on = false;
        let txt = build_txt(&info);
        assert_eq!(txt.get("vpn").map(|s| s.as_str()), Some("off"));
    }

    #[test]
    fn parse_falls_back_on_missing_pac() {
        let mut txt = build_txt(&sample());
        txt.remove("pac");
        let parsed = parse_txt(&txt, 0).unwrap();
        assert_eq!(parsed.pac_path, DEFAULT_PAC_PATH);
    }

    #[test]
    fn parse_falls_back_to_provided_http_port_when_txt_missing() {
        let mut txt = build_txt(&sample());
        txt.remove("port");
        let parsed = parse_txt(&txt, 9999).unwrap();
        assert_eq!(parsed.http_port, 9999);
    }

    #[test]
    fn parse_errors_on_invalid_port() {
        let mut txt = build_txt(&sample());
        txt.insert("port".into(), "not-a-number".into());
        match parse_txt(&txt, 0) {
            Err(MdnsParseError::InvalidField { field, .. }) => assert_eq!(field, "port"),
            other => panic!("expected InvalidField, got {other:?}"),
        }
    }

    #[test]
    fn parse_errors_when_name_missing() {
        let mut txt = build_txt(&sample());
        txt.remove("name");
        assert_eq!(parse_txt(&txt, 0), Err(MdnsParseError::MissingField("name")));
    }

    #[test]
    fn instance_fqdn_uses_conduit_on_prefix() {
        let info = sample();
        assert_eq!(info.instance_fqdn(), "Conduit on host01._conduit._tcp.local.");
        assert_eq!(info.server_fqdn(), "host01.local.");
    }
}
