//! 通用错误模型，server-app / client-app / 各模块共享。
//!
//! 所有可恢复错误都收敛到 [`ConduitError`]。Tauri command 返回时
//! 通过 `From<ConduitError> for String` 暴露给 UI（保留 thiserror 文案）。
//!
//! 设计参考：`design/2026-05-06-2-Conduit-Rust-重写设计文档.md` §10。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConduitError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid address: {0}")]
    InvalidAddr(String),

    #[error("upstream: {0}")]
    Upstream(String),

    #[error("pac parse: {0}")]
    PacParse(String),

    #[error("mdns: {0}")]
    Mdns(String),

    #[error("system_proxy: {0}")]
    SystemProxy(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("internal: {0}")]
    Internal(String),
}

pub type ConduitResult<T> = Result<T, ConduitError>;

impl From<ConduitError> for String {
    fn from(err: ConduitError) -> Self {
        err.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_matches_variants() {
        let cases = [
            (ConduitError::InvalidAddr("1.2.3.x".into()), "invalid address: 1.2.3.x"),
            (ConduitError::Upstream("dns: NXDOMAIN".into()), "upstream: dns: NXDOMAIN"),
            (ConduitError::PacParse("missing section 3".into()), "pac parse: missing section 3"),
            (ConduitError::Mdns("daemon down".into()), "mdns: daemon down"),
            (ConduitError::SystemProxy("networksetup -2".into()), "system_proxy: networksetup -2"),
            (ConduitError::NotFound("server foo".into()), "not found: server foo"),
            (ConduitError::Forbidden("not in allowed_cidrs".into()), "forbidden: not in allowed_cidrs"),
            (ConduitError::Internal("unreachable".into()), "internal: unreachable"),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn io_error_auto_converts_via_from() {
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "test");
        let conduit_err: ConduitError = io_err.into();
        assert!(conduit_err.to_string().starts_with("io: "));
        assert!(conduit_err.to_string().contains("test"));
    }

    #[test]
    fn into_string_for_tauri_command() {
        let err = ConduitError::NotFound("server xyz".into());
        let s: String = err.into();
        assert_eq!(s, "not found: server xyz");
    }
}
