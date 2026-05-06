//! client-app boot 期错误。
//!
//! 与 [`conduit_core::ConduitError`]（业务侧错误）分开：本 enum 只表达
//! 启动阶段会出现的错误（healthz 超时 / 端口分配失败 / IO / 兜底 internal）。
//! Tauri command 返回时通过自定义 `Serialize` 输出 `{code, message}` 给 UI。

use serde::{Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConduitError {
    #[error("control api healthz timeout after {0}s")]
    HealthzTimeout(u64),

    #[error("port allocation failed")]
    #[allow(dead_code)]
    PortAlloc,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("internal: {0}")]
    Internal(String),

    /// dev 模式下 `restart_app` 不支持（vite 与 binary 解耦）。
    /// UI 单独识别这个 code 走 toast.info（友好提示），不要当成报错。
    #[error("dev restart unsupported: {0}")]
    DevRestartUnsupported(String),
}

impl Serialize for ConduitError {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let code = match self {
            ConduitError::HealthzTimeout(_) => "HEALTHZ_TIMEOUT",
            ConduitError::PortAlloc => "PORT_ALLOC",
            ConduitError::Io(_) => "IO",
            ConduitError::Internal(_) => "INTERNAL",
            ConduitError::DevRestartUnsupported(_) => "DEV_RESTART_UNSUPPORTED",
        };
        let mut state = ser.serialize_struct("ConduitError", 2)?;
        use serde::ser::SerializeStruct;
        state.serialize_field("code", code)?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

pub type ConduitResult<T> = Result<T, ConduitError>;
