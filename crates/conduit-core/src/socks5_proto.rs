//! RFC 1928 SOCKS5 协议字节级编解码（双端共享）。
//!
//! 设计取舍：
//! - **只下沉纯字节层**：常量、`Address` 枚举、各帧的 encode/decode buffer 拼装。
//! - **不下沉 IO**：server 端有 per-step timeout、ACL 校验插在中间；
//!   client 端有连接超时、嵌套握手等差异。把 `AsyncRead` 抽象到 core 反而
//!   把这些差异硬编码进来，得不偿失。
//! - 所有 `parse_*` 函数都接收已读好的 `&[u8]` slice，不依赖 tokio。
//! - 上层（server-app/socks5.rs、client-app/local_proxy.rs）继续负责
//!   "用 timeout 读 N 字节、然后调本模块 parse"。
//!
//! 参考：[RFC 1928](https://datatracker.ietf.org/doc/html/rfc1928)。

use std::net::{Ipv4Addr, Ipv6Addr};

use thiserror::Error;

/// SOCKS5 协议常量。
pub mod consts {
    pub const VER: u8 = 0x05;
    pub const NO_AUTH: u8 = 0x00;
    pub const NO_ACCEPTABLE: u8 = 0xFF;

    pub const CMD_CONNECT: u8 = 0x01;

    pub const ATYP_IPV4: u8 = 0x01;
    pub const ATYP_DOMAIN: u8 = 0x03;
    pub const ATYP_IPV6: u8 = 0x04;

    pub const REP_OK: u8 = 0x00;
    pub const REP_GENERAL: u8 = 0x01;
    pub const REP_NOT_ALLOWED: u8 = 0x02;
    pub const REP_NETWORK_UNREACH: u8 = 0x03;
    pub const REP_CONNECT_REFUSED: u8 = 0x05;
    pub const REP_TTL_EXPIRED: u8 = 0x06;
    pub const REP_CMD_NOT_SUPPORTED: u8 = 0x07;
    pub const REP_ATYP_NOT_SUPPORTED: u8 = 0x08;
}

/// SOCKS5 地址（对应 ATYP IPv4 / IPv6 / DOMAIN）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Socks5Address {
    V4([u8; 4]),
    V6([u8; 16]),
    Domain(String),
}

impl Socks5Address {
    /// 返回对应 ATYP 字节。
    pub fn atyp(&self) -> u8 {
        match self {
            Self::V4(_) => consts::ATYP_IPV4,
            Self::V6(_) => consts::ATYP_IPV6,
            Self::Domain(_) => consts::ATYP_DOMAIN,
        }
    }

    /// 返回可用于 `TcpStream::connect` 的 host 字符串。
    ///
    /// - V4 → `1.2.3.4`
    /// - V6 → `::1`
    /// - Domain → 原始 host
    pub fn host_string(&self) -> String {
        match self {
            Self::V4(b) => Ipv4Addr::from(*b).to_string(),
            Self::V6(b) => Ipv6Addr::from(*b).to_string(),
            Self::Domain(s) => s.clone(),
        }
    }
}

/// 协议层错误（不含 IO 错误）。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Socks5ProtoError {
    #[error("invalid socks version: 0x{0:02x}")]
    InvalidVersion(u8),
    #[error("server did not accept NO-AUTH (got 0x{0:02x})")]
    NoAuthRejected(u8),
    #[error("server reply rep=0x{0:02x}")]
    NonOkReply(u8),
    #[error("invalid atyp: 0x{0:02x}")]
    InvalidAtyp(u8),
    #[error("domain too long: {0} bytes (max 255)")]
    DomainTooLong(usize),
    #[error("buffer truncated: need {need} bytes, got {got}")]
    Truncated { need: usize, got: usize },
}

// ─────────────────────── encoders（拼字节 → Vec / array）───────────────────────

/// Client 侧：方法协商请求 `[VER, NMETHODS=1, NO_AUTH]`。
pub fn encode_method_request_no_auth() -> [u8; 3] {
    [consts::VER, 1, consts::NO_AUTH]
}

/// Server 侧：方法响应 `[VER, NO_AUTH | NO_ACCEPTABLE]`。
pub fn encode_method_response(no_auth_ok: bool) -> [u8; 2] {
    [
        consts::VER,
        if no_auth_ok {
            consts::NO_AUTH
        } else {
            consts::NO_ACCEPTABLE
        },
    ]
}

/// Client 侧：CONNECT 请求 `[VER, CMD_CONNECT, RSV=0, ATYP, ADDR..., PORT(BE)]`。
pub fn encode_connect_request(
    addr: &Socks5Address,
    port: u16,
) -> Result<Vec<u8>, Socks5ProtoError> {
    let mut buf = Vec::with_capacity(22);
    buf.push(consts::VER);
    buf.push(consts::CMD_CONNECT);
    buf.push(0x00);
    buf.push(addr.atyp());
    match addr {
        Socks5Address::V4(b) => buf.extend_from_slice(b),
        Socks5Address::V6(b) => buf.extend_from_slice(b),
        Socks5Address::Domain(s) => {
            let bytes = s.as_bytes();
            if bytes.len() > 255 {
                return Err(Socks5ProtoError::DomainTooLong(bytes.len()));
            }
            buf.push(bytes.len() as u8);
            buf.extend_from_slice(bytes);
        }
    }
    buf.extend_from_slice(&port.to_be_bytes());
    Ok(buf)
}

/// Server 侧：reply 帧 `[VER, REP, RSV=0, ATYP, BND.ADDR, BND.PORT(BE)]`。
///
/// `bnd_addr` / `bnd_port` 在错误响应中可填全 0（约定俗成）。
pub fn encode_reply(rep: u8, bnd_atyp: u8, bnd_addr: &[u8], bnd_port: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + bnd_addr.len());
    out.push(consts::VER);
    out.push(rep);
    out.push(0x00);
    out.push(bnd_atyp);
    out.extend_from_slice(bnd_addr);
    out.extend_from_slice(&bnd_port.to_be_bytes());
    out
}

/// Server 侧便捷方法：错误 reply（BND.ADDR 全 0、BND.PORT=0）。
pub fn encode_error_reply(rep: u8) -> Vec<u8> {
    encode_reply(rep, consts::ATYP_IPV4, &[0, 0, 0, 0], 0)
}

// ─────────────────────── parsers（在已读 slice 上解码）───────────────────────

/// 校验响应版本字节是否为 `0x05`。
pub fn validate_version(byte: u8) -> Result<(), Socks5ProtoError> {
    if byte == consts::VER {
        Ok(())
    } else {
        Err(Socks5ProtoError::InvalidVersion(byte))
    }
}

/// Client 侧：解析方法响应 `[VER, METHOD]`，要求 NO-AUTH。
pub fn parse_method_response(buf: &[u8; 2]) -> Result<(), Socks5ProtoError> {
    validate_version(buf[0])?;
    if buf[1] != consts::NO_AUTH {
        return Err(Socks5ProtoError::NoAuthRejected(buf[1]));
    }
    Ok(())
}

/// Client 侧：解析 reply 头 4 字节 `[VER, REP, RSV, ATYP]`，返回 `(rep, atyp)`。
///
/// 仅校验 VER 与 rep == 0；不读 BND.ADDR / BND.PORT（长度依赖 atyp，由调用者继续读）。
pub fn parse_reply_head(buf: &[u8; 4]) -> Result<u8, Socks5ProtoError> {
    validate_version(buf[0])?;
    let rep = buf[1];
    if rep != consts::REP_OK {
        return Err(Socks5ProtoError::NonOkReply(rep));
    }
    Ok(buf[3])
}

/// 给定 ATYP，返回 BND.ADDR 字段需要继续读的字节数。
///
/// - IPv4 → 4
/// - IPv6 → 16
/// - DOMAIN → 调用方需先读 1 字节长度再按值读
/// - 其它 → `InvalidAtyp`
pub fn bnd_addr_len(atyp: u8) -> Result<Option<usize>, Socks5ProtoError> {
    match atyp {
        consts::ATYP_IPV4 => Ok(Some(4)),
        consts::ATYP_IPV6 => Ok(Some(16)),
        consts::ATYP_DOMAIN => Ok(None),
        other => Err(Socks5ProtoError::InvalidAtyp(other)),
    }
}

/// 把已读完整的 `bnd_addr` slice 解析成 `Socks5Address`。Domain 字段调用方需先剥掉长度字节。
pub fn parse_address_bytes(atyp: u8, bytes: &[u8]) -> Result<Socks5Address, Socks5ProtoError> {
    match atyp {
        consts::ATYP_IPV4 => {
            if bytes.len() != 4 {
                return Err(Socks5ProtoError::Truncated {
                    need: 4,
                    got: bytes.len(),
                });
            }
            let mut b = [0u8; 4];
            b.copy_from_slice(bytes);
            Ok(Socks5Address::V4(b))
        }
        consts::ATYP_IPV6 => {
            if bytes.len() != 16 {
                return Err(Socks5ProtoError::Truncated {
                    need: 16,
                    got: bytes.len(),
                });
            }
            let mut b = [0u8; 16];
            b.copy_from_slice(bytes);
            Ok(Socks5Address::V6(b))
        }
        consts::ATYP_DOMAIN => Ok(Socks5Address::Domain(
            String::from_utf8_lossy(bytes).into_owned(),
        )),
        other => Err(Socks5ProtoError::InvalidAtyp(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_atyp_matches_variant() {
        assert_eq!(Socks5Address::V4([0, 0, 0, 0]).atyp(), consts::ATYP_IPV4);
        assert_eq!(Socks5Address::V6([0; 16]).atyp(), consts::ATYP_IPV6);
        assert_eq!(
            Socks5Address::Domain("example.com".into()).atyp(),
            consts::ATYP_DOMAIN
        );
    }

    #[test]
    fn address_host_string_renders_like_std() {
        assert_eq!(Socks5Address::V4([10, 0, 0, 1]).host_string(), "10.0.0.1");
        assert_eq!(
            Socks5Address::V6([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]).host_string(),
            "::1"
        );
        assert_eq!(
            Socks5Address::Domain("a.example.com".into()).host_string(),
            "a.example.com"
        );
    }

    #[test]
    fn method_request_is_canonical_three_bytes() {
        assert_eq!(encode_method_request_no_auth(), [0x05, 0x01, 0x00]);
    }

    #[test]
    fn method_response_encodes_both_branches() {
        assert_eq!(encode_method_response(true), [0x05, 0x00]);
        assert_eq!(encode_method_response(false), [0x05, 0xFF]);
    }

    #[test]
    fn parse_method_response_accepts_no_auth() {
        parse_method_response(&[0x05, 0x00]).unwrap();
    }

    #[test]
    fn parse_method_response_rejects_no_acceptable() {
        let err = parse_method_response(&[0x05, 0xFF]).unwrap_err();
        assert_eq!(err, Socks5ProtoError::NoAuthRejected(0xFF));
    }

    #[test]
    fn parse_method_response_rejects_bad_version() {
        let err = parse_method_response(&[0x04, 0x00]).unwrap_err();
        assert_eq!(err, Socks5ProtoError::InvalidVersion(0x04));
    }

    #[test]
    fn connect_request_v4_layout() {
        let bytes = encode_connect_request(&Socks5Address::V4([10, 0, 0, 1]), 80).unwrap();
        assert_eq!(
            bytes,
            vec![0x05, 0x01, 0x00, 0x01, 10, 0, 0, 1, 0x00, 0x50]
        );
    }

    #[test]
    fn connect_request_v6_layout() {
        let bytes = encode_connect_request(
            &Socks5Address::V6([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
            443,
        )
        .unwrap();
        assert_eq!(bytes[0..4], [0x05, 0x01, 0x00, 0x04]);
        assert_eq!(&bytes[4..20], &[0u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(&bytes[20..22], &[0x01, 0xBB]);
    }

    #[test]
    fn connect_request_domain_prefixes_length() {
        let bytes = encode_connect_request(&Socks5Address::Domain("a.b".into()), 8080).unwrap();
        // [VER CMD RSV ATYP LEN 'a' '.' 'b' PORT_HI PORT_LO]
        assert_eq!(
            bytes,
            vec![0x05, 0x01, 0x00, 0x03, 3, b'a', b'.', b'b', 0x1F, 0x90]
        );
    }

    #[test]
    fn connect_request_rejects_overlong_domain() {
        let host = "a".repeat(256);
        let err = encode_connect_request(&Socks5Address::Domain(host), 80).unwrap_err();
        assert_eq!(err, Socks5ProtoError::DomainTooLong(256));
    }

    #[test]
    fn reply_layout_is_six_plus_addr_bytes() {
        let bytes = encode_reply(consts::REP_OK, consts::ATYP_IPV4, &[127, 0, 0, 1], 8443);
        assert_eq!(bytes, vec![0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0x20, 0xFB]);
    }

    #[test]
    fn error_reply_zeroes_out_addr() {
        assert_eq!(
            encode_error_reply(consts::REP_NOT_ALLOWED),
            vec![0x05, 0x02, 0x00, 0x01, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn parse_reply_head_returns_atyp_when_ok() {
        let atyp = parse_reply_head(&[0x05, 0x00, 0x00, 0x03]).unwrap();
        assert_eq!(atyp, consts::ATYP_DOMAIN);
    }

    #[test]
    fn parse_reply_head_rejects_non_zero_rep() {
        let err = parse_reply_head(&[0x05, 0x05, 0x00, 0x01]).unwrap_err();
        assert_eq!(err, Socks5ProtoError::NonOkReply(0x05));
    }

    #[test]
    fn bnd_addr_len_dispatches_by_atyp() {
        assert_eq!(bnd_addr_len(consts::ATYP_IPV4).unwrap(), Some(4));
        assert_eq!(bnd_addr_len(consts::ATYP_IPV6).unwrap(), Some(16));
        assert_eq!(bnd_addr_len(consts::ATYP_DOMAIN).unwrap(), None);
        assert_eq!(
            bnd_addr_len(0xFE).unwrap_err(),
            Socks5ProtoError::InvalidAtyp(0xFE)
        );
    }

    #[test]
    fn parse_address_bytes_roundtrip_ipv4() {
        let addr = parse_address_bytes(consts::ATYP_IPV4, &[10, 0, 0, 9]).unwrap();
        assert_eq!(addr, Socks5Address::V4([10, 0, 0, 9]));
        assert_eq!(addr.host_string(), "10.0.0.9");
    }

    #[test]
    fn parse_address_bytes_roundtrip_ipv6() {
        let raw = [0u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let addr = parse_address_bytes(consts::ATYP_IPV6, &raw).unwrap();
        assert_eq!(addr, Socks5Address::V6(raw));
    }

    #[test]
    fn parse_address_bytes_truncated_returns_err() {
        let err = parse_address_bytes(consts::ATYP_IPV4, &[10, 0, 0]).unwrap_err();
        assert_eq!(err, Socks5ProtoError::Truncated { need: 4, got: 3 });
    }

    #[test]
    fn parse_address_bytes_decodes_domain_lossy() {
        let addr = parse_address_bytes(consts::ATYP_DOMAIN, b"example.com").unwrap();
        assert_eq!(addr, Socks5Address::Domain("example.com".into()));
    }
}
