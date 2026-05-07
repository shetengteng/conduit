//! 双端 boot 阶段错误（server-app / client-app 启动期共享）。
//!
//! 与 [`crate::error::ConduitError`]（业务侧错误）分开：本 enum 只表达
//! 启动阶段会出现的错误（healthz 超时 / 端口分配失败 / IO / 兜底 internal）。
//!
//! Tauri command 返回时通过自定义 `Serialize` 输出 `{code, message}` 给 UI。
//! 双端 wire-format 100% 一致：UI 端依赖 `code` 字段做分支（如
//! `DEV_RESTART_UNSUPPORTED` → toast.info 而非 toast.error）。
//!
//! ## 用法
//!
//! 双端 `crate::error` 模块 re-export 本类型即可：
//!
//! ```ignore
//! pub use conduit_core::boot_error::{BootError as ConduitError, BootResult as ConduitResult};
//! ```
//!
//! 这样保持双端 `crate::error::ConduitError` 调用点零修改，只是底层共享一份实现。
//!
//! ## 历史
//!
//! v0.2.0 之前，server-app/src/error.rs 与 client-app/src/error.rs 是 100% 重复的
//! 两份独立 enum。v0.2.0 W6 后续整理（2026-05-07）下沉到本模块，去重 ~95 行。

use serde::{Serialize, Serializer};
use thiserror::Error;

/// 双端 boot 阶段错误。
///
/// 与 `conduit_core::ConduitError`（业务侧）分开：本 enum 只表达启动阶段错误。
#[derive(Debug, Error)]
pub enum BootError {
    /// 控制 API healthz 在指定秒数内仍未就绪。
    #[error("control api healthz timeout after {0}s")]
    HealthzTimeout(u64),

    /// 端口分配失败（OS 无可用端口）。
    #[error("port allocation failed")]
    PortAlloc,

    /// 其他 IO 错误（透传 `std::io::Error`）。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// 兜底内部错误（不属于其他分支的字符串原因）。
    #[error("internal: {0}")]
    Internal(String),

    /// dev 模式下 `restart_app` 不支持（vite 与 binary 解耦）。
    /// UI 单独识别这个 code 走 toast.info（友好提示），不要当成报错。
    #[error("dev restart unsupported: {0}")]
    DevRestartUnsupported(String),
}

impl Serialize for BootError {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let code = match self {
            BootError::HealthzTimeout(_) => "HEALTHZ_TIMEOUT",
            BootError::PortAlloc => "PORT_ALLOC",
            BootError::Io(_) => "IO",
            BootError::Internal(_) => "INTERNAL",
            BootError::DevRestartUnsupported(_) => "DEV_RESTART_UNSUPPORTED",
        };
        let mut state = ser.serialize_struct("BootError", 2)?;
        use serde::ser::SerializeStruct;
        state.serialize_field("code", code)?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

/// `Result<T, BootError>` 的别名，双端业务代码统一使用这个 typedef。
pub type BootResult<T> = Result<T, BootError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn ser(err: BootError) -> serde_json::Value {
        serde_json::to_value(err).expect("BootError 必须能序列化为 JSON")
    }

    #[test]
    fn healthz_timeout_serializes_with_seconds_in_message() {
        let v = ser(BootError::HealthzTimeout(15));
        assert_eq!(v["code"], "HEALTHZ_TIMEOUT");
        assert_eq!(v["message"], "control api healthz timeout after 15s");
    }

    #[test]
    fn port_alloc_serializes_with_stable_code() {
        let v = ser(BootError::PortAlloc);
        assert_eq!(v["code"], "PORT_ALLOC");
        assert_eq!(v["message"], "port allocation failed");
    }

    #[test]
    fn io_error_passthrough_message() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "oops");
        let v = ser(BootError::Io(io_err));
        assert_eq!(v["code"], "IO");
        assert!(v["message"].as_str().unwrap().contains("oops"));
    }

    #[test]
    fn internal_keeps_caller_supplied_message() {
        let v = ser(BootError::Internal("config invalid".into()));
        assert_eq!(v["code"], "INTERNAL");
        assert_eq!(v["message"], "internal: config invalid");
    }

    #[test]
    fn dev_restart_unsupported_uses_friendly_code() {
        let v = ser(BootError::DevRestartUnsupported("vite running".into()));
        assert_eq!(v["code"], "DEV_RESTART_UNSUPPORTED");
        assert_eq!(v["message"], "dev restart unsupported: vite running");
    }

    #[test]
    fn from_io_error_works_via_question_mark() {
        fn inner() -> BootResult<()> {
            std::fs::read_to_string("/definitely/does/not/exist/conduit-test")?;
            Ok(())
        }
        let err = inner().expect_err("不存在的文件读取必须出错");
        match err {
            BootError::Io(_) => {}
            other => panic!("expected BootError::Io, got {other:?}"),
        }
    }
}
