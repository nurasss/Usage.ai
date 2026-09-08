use super::HostError;
use async_trait::async_trait;
use std::time::Duration;

pub struct HttpJsonRequest<'a> {
    pub url: &'a str,
    pub bearer: Option<&'a [u8]>,
    pub allowed_hosts: &'a [&'a str],
    pub timeout: Duration,
    pub max_bytes: usize,
    pub user_agent: &'a str,
}

pub struct HttpJsonResponse {
    pub status: u16,
    pub retry_after_secs: Option<u64>,
    pub body: Option<serde_json::Value>,
}

/// Secret-bearing JSON GET with enforced policy:
/// HTTPS only, exact-host allowlist, redirects denied (any 3xx is an
/// error, credentials are never forwarded), per-request timeout,
/// response size limit, safe user agent, no auth data in errors/logs.
#[async_trait]
pub trait HttpHost: Send + Sync {
    async fn get_json(&self, req: HttpJsonRequest<'_>) -> Result<HttpJsonResponse, HostError>;
}

#[derive(Debug, Clone)]
pub struct ReqwestHttpHost {
    max_bytes_default: usize,
}

impl Default for ReqwestHttpHost {
    fn default() -> Self {
        Self {
            max_bytes_default: super::policy::MAX_HTTP_BYTES,
        }
    }
}

#[async_trait]
impl HttpHost for ReqwestHttpHost {
    async fn get_json(&self, req: HttpJsonRequest<'_>) -> Result<HttpJsonResponse, HostError> {
        let url: reqwest::Url = req.url.parse().map_err(|_| HostError::Policy)?;
        if url.scheme() != "https" {
            return Err(HostError::Policy);
        }
        let host = url.host_str().unwrap_or_default();
        if !req.allowed_hosts.contains(&host) {
            return Err(HostError::Policy);
        }
        let limit = req.max_bytes.min(self.max_bytes_default).max(1024);
        let client = reqwest::Client::builder()
            .timeout(req.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(req.user_agent)
            .build()
            .map_err(|_| HostError::Unavailable)?;
        let mut builder = client.get(url);
        if let Some(secret) = req.bearer {
            let token = String::from_utf8(secret.to_vec()).map_err(|_| HostError::Policy)?;
            builder = builder.bearer_auth(token);
        }
        let response = builder.send().await.map_err(|e| {
            if e.is_timeout() {
                HostError::Timeout
            } else {
                HostError::Unavailable
            }
        })?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(HostError::Policy);
        }
        // Header value is numeric backoff metadata, never a secret.
        let retry_after_secs = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| parse_retry_after(Some(v)));
        if let Some(len) = response.content_length() {
            if len > limit as u64 {
                return Err(HostError::Policy);
            }
        }
        if status == 401 {
            return Err(HostError::Denied);
        }
        if status == 403 {
            return Err(HostError::Forbidden);
        }
        if status == 429 {
            return Err(HostError::RateLimited(retry_after_secs));
        }
        if !(200..300).contains(&status) {
            return Err(HostError::Unavailable);
        }
        let bytes = response.bytes().await.map_err(|e| {
            if e.is_timeout() {
                HostError::Timeout
            } else {
                HostError::Unavailable
            }
        })?;
        if bytes.len() > limit {
            return Err(HostError::Policy);
        }
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| HostError::Unavailable)?;
        Ok(HttpJsonResponse {
            status,
            retry_after_secs,
            body: Some(body),
        })
    }
}

/// Extract `Retry-After` seconds from a raw header value without logging it.
pub fn parse_retry_after(value: Option<&str>) -> Option<u64> {
    value?.trim().parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_plain_http_before_any_network() {
        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: "http://api.openai.com/v1/x",
                bearer: None,
                allowed_hosts: &["api.openai.com"],
                timeout: Duration::from_secs(5),
                max_bytes: 1024,
                user_agent: "Usage.ai-test",
            })
            .await;
        assert!(matches!(result, Err(HostError::Policy)));
    }

    #[tokio::test]
    async fn rejects_non_allowlisted_host_before_any_network() {
        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: "https://evil.example.com/v1/x",
                bearer: None,
                allowed_hosts: &["api.openai.com"],
                timeout: Duration::from_secs(5),
                max_bytes: 1024,
                user_agent: "Usage.ai-test",
            })
            .await;
        assert!(matches!(result, Err(HostError::Policy)));
    }

    #[test]
    fn retry_after_parses_digits_only() {
        assert_eq!(parse_retry_after(Some("77")), Some(77));
        assert_eq!(parse_retry_after(Some("soon")), None);
        assert_eq!(parse_retry_after(None), None);
    }
}
