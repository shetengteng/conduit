use serde::{Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConduitError {
    #[error("sidecar spawn failed: {0}")]
    SidecarSpawn(String),

    #[error("sidecar healthz timeout after {0}s")]
    HealthzTimeout(u64),

    #[error("port allocation failed")]
    #[allow(dead_code)]
    PortAlloc,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("internal: {0}")]
    Internal(String),
}

impl Serialize for ConduitError {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let code = match self {
            ConduitError::SidecarSpawn(_) => "SIDECAR_SPAWN",
            ConduitError::HealthzTimeout(_) => "HEALTHZ_TIMEOUT",
            ConduitError::PortAlloc => "PORT_ALLOC",
            ConduitError::Io(_) => "IO",
            ConduitError::Http(_) => "HTTP",
            ConduitError::Internal(_) => "INTERNAL",
        };
        let mut state = ser.serialize_struct("ConduitError", 2)?;
        use serde::ser::SerializeStruct;
        state.serialize_field("code", code)?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

pub type ConduitResult<T> = Result<T, ConduitError>;
