//! Cloudflare Turnstile Siteverify 的进程级 HTTP adapter。

use std::{net::IpAddr, time::Duration};

use async_trait::async_trait;
use gateway_admin::ports::auth::{TurnstileVerifier, TurnstileVerifyError};
use serde::Deserialize;

const SITEVERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

pub struct HttpTurnstileVerifier {
    client: reqwest::Client,
}

impl HttpTurnstileVerifier {
    pub fn new() -> Result<Self, TurnstileClientError> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| TurnstileClientError)?;
        Ok(Self { client })
    }
}

#[derive(serde::Serialize)]
struct SiteverifyForm<'a> {
    secret: &'a str,
    response: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    remoteip: Option<IpAddr>,
}

#[derive(Deserialize)]
struct SiteverifyResponse {
    success: bool,
}

#[async_trait]
impl TurnstileVerifier for HttpTurnstileVerifier {
    async fn verify(
        &self,
        secret: &str,
        token: &str,
        remote_ip: Option<IpAddr>,
    ) -> Result<bool, TurnstileVerifyError> {
        let response = self
            .client
            .post(SITEVERIFY_URL)
            .form(&SiteverifyForm {
                secret,
                response: token,
                remoteip: remote_ip,
            })
            .send()
            .await
            .map_err(|_| TurnstileVerifyError::Unavailable)?;
        if !response.status().is_success() {
            return Err(TurnstileVerifyError::Unavailable);
        }
        response
            .json::<SiteverifyResponse>()
            .await
            .map(|body| body.success)
            .map_err(|_| TurnstileVerifyError::Unavailable)
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("failed to initialize Turnstile HTTP client")]
pub struct TurnstileClientError;
