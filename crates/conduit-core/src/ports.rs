//! `ports` —— 通用端口分配 helper，server-app / client-app 共用。
//!
//! 之前 server-app 用 `pick_three_ports`、client-app 用 `pick_two_ports`，
//! 仅参数 N 不同。下沉到 conduit-core::ports::pick_unused_ports(n)，
//! 内部仍走 [`portpicker`] 拿 ephemeral port，但保证 N 个端口互不重复。

/// 分配 `n` 个互不重复的可用 TCP 端口。
///
/// 返回 `Some(vec![p1, p2, ..., pn])` 表示成功；任一端口 32 次内拿不到就返回 `None`。
///
/// 实现细节：
/// - 用 [`portpicker::pick_unused_port`] 拿候选，过滤掉已分配过的，避免 N 个相同。
/// - 32 次重试上限避免极端竞态死循环。
pub fn pick_unused_ports(n: usize) -> Option<Vec<u16>> {
    let mut taken = Vec::with_capacity(n);
    for _ in 0..n {
        let mut found = None;
        for _ in 0..32 {
            match portpicker::pick_unused_port() {
                Some(p) if !taken.contains(&p) => {
                    found = Some(p);
                    break;
                }
                _ => continue,
            }
        }
        match found {
            Some(p) => taken.push(p),
            None => return None,
        }
    }
    Some(taken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn picks_three_distinct_ports() {
        let ports = pick_unused_ports(3).expect("should find three ports");
        assert_eq!(ports.len(), 3);
        let set: HashSet<u16> = ports.iter().copied().collect();
        assert_eq!(set.len(), 3, "expected 3 distinct ports, got {ports:?}");
        for p in ports {
            assert!(p >= 1024, "expected ephemeral port, got {p}");
        }
    }

    #[test]
    fn picks_two_distinct_ports() {
        let ports = pick_unused_ports(2).expect("should find two ports");
        assert_eq!(ports.len(), 2);
        assert_ne!(ports[0], ports[1]);
    }

    #[test]
    fn zero_returns_empty_vec() {
        let ports = pick_unused_ports(0).unwrap();
        assert!(ports.is_empty());
    }
}
