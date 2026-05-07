//! 时间戳辅助。
//!
//! 之前 server-app 内部 `session.rs` / `control_api.rs` / `core.rs` /
//! `traffic_emitter.rs` 各写一份相同的 `epoch_secs() -> f64`,这里下沉到
//! conduit-core 让两端共享同一份实现。
//!
//! 不引 chrono: 这个函数仅用作 wire 字段(snake_case JSON,UI 端用 number
//! 直接消费),用 std::time 即可,避免 chrono 全量编译开销。

use std::time::{SystemTime, UNIX_EPOCH};

/// 返回当前 UNIX epoch 秒(浮点,小数部分精度到毫秒/微秒级)。
///
/// 系统时钟早于 epoch 时(几乎不可能,但理论上发生过)返回 0.0。
pub fn epoch_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_secs_is_monotonic_non_negative() {
        let a = epoch_secs();
        let b = epoch_secs();
        assert!(a >= 0.0);
        assert!(b >= a, "subsequent epoch_secs should not go backwards: {a} → {b}");
    }

    #[test]
    fn epoch_secs_rough_year_sanity() {
        // 2025-01-01 = 1735689600 (epoch sec). 测试运行时点必然晚于此。
        assert!(epoch_secs() > 1_735_689_600.0);
    }
}
