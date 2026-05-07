//! server-app boot 期错误（thin re-export）。
//!
//! v0.2.0 W6 后续整理（2026-05-07）后，本类型实际定义在
//! [`conduit_core::boot_error`]，与 client-app/src/error.rs 共享一份实现，
//! 避免双端 100% 重复 enum。
//!
//! 调用方继续 `use crate::error::ConduitError` 即可，无需改动。
//! Wire-format（`{code, message}`）100% 不变。

pub use conduit_core::boot_error::BootError as ConduitError;
#[allow(unused_imports)]
pub use conduit_core::boot_error::BootResult as ConduitResult;
