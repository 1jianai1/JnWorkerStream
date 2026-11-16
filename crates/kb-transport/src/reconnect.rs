//! 断线自动重连的退避策略：2 s 起，×2，封顶 30 s。

/// 重连退避配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectPolicy {
    /// 是否自动重连（可由 `link.reconnect_policy` 关闭）。
    pub auto: bool,
    /// 首次退避。
    pub base_ms: u64,
    /// 封顶退避。
    pub max_ms: u64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            auto: true,
            base_ms: 2_000,
            max_ms: 30_000,
        }
    }
}

impl ReconnectPolicy {
    /// 第 `attempt` 次重试前的等待（`attempt` 从 1 开始）。
    ///
    /// `attempt = 0` 视为“立即重试”；指数溢出时直接封顶。
    pub fn delay_ms(&self, attempt: u32) -> u64 {
        if attempt == 0 {
            return 0;
        }
        let shift = (attempt - 1).min(16);
        self.base_ms.saturating_mul(1u64 << shift).min(self.max_ms)
    }

    /// 本次是否应该自动重连。
    pub fn should_retry(&self) -> bool {
        self.auto
    }
}

#[cfg(test)]
mod tests {
    use super::ReconnectPolicy;

    #[test]
    fn backoff_doubles_until_cap() {
        let policy = ReconnectPolicy::default();
        assert_eq!(policy.delay_ms(0), 0);
        assert_eq!(policy.delay_ms(1), 2_000);
        assert_eq!(policy.delay_ms(2), 4_000);
        assert_eq!(policy.delay_ms(3), 8_000);
        assert_eq!(policy.delay_ms(4), 16_000);
        assert_eq!(policy.delay_ms(5), 30_000);
        assert_eq!(policy.delay_ms(64), 30_000);
    }

    #[test]
    fn auto_can_be_disabled() {
        let mut policy = ReconnectPolicy::default();
        assert!(policy.should_retry());
        policy.auto = false;
        assert!(!policy.should_retry());
    }
}
