//! 登录防护依赖的外部验证端口。

use std::net::IpAddr;

use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TurnstileVerifyError {
    #[error("turnstile verification is unavailable")]
    Unavailable,
}

/// Cloudflare Turnstile Siteverify 的最小抽象；secret/token 不进入错误或日志。
#[async_trait]
pub trait TurnstileVerifier: Send + Sync {
    async fn verify(
        &self,
        secret: &str,
        token: &str,
        remote_ip: Option<IpAddr>,
    ) -> Result<bool, TurnstileVerifyError>;
}
