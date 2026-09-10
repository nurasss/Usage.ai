use super::{CancellationToken, HostError};
use async_trait::async_trait;
use std::time::Duration;

pub struct HttpJsonRequest<'a> {
    pub url: &'a str,
    pub bearer: Option<&'a [u8]>,
    pub allowed_hosts: &'a [&'a str],
    pub timeout: Duration,
    pub max_bytes: usize,
    pub user_agent: &'a str,
    pub cancel: CancellationToken,
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

#[derive(Debug)]
pub struct ReqwestHttpHost {
    max_bytes_default: usize,
    client: tokio::sync::OnceCell<reqwest::Client>,
}

impl Default for ReqwestHttpHost {
    fn default() -> Self {
        Self {
            max_bytes_default: super::policy::MAX_HTTP_BYTES,
            client: tokio::sync::OnceCell::new(),
        }
    }
}

impl ReqwestHttpHost {
    pub fn new(max_bytes_default: usize) -> Self {
        Self {
            max_bytes_default: if max_bytes_default > 0 {
                max_bytes_default
            } else {
                super::policy::MAX_HTTP_BYTES
            },
            client: tokio::sync::OnceCell::new(),
        }
    }

    /// One reusable pooled client per host: connection reuse without
    /// per-request rebuilds. Redirects stay denied and the safe
    /// user agent is fixed at construction.
    async fn client(&self) -> Result<&reqwest::Client, HostError> {
        self.client
            .get_or_try_init(|| async {
                reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .user_agent(super::policy::USER_AGENT)
                    .build()
                    .map_err(|_| HostError::Unavailable)
            })
            .await
    }
}

#[async_trait]
impl HttpHost for ReqwestHttpHost {
    async fn get_json(&self, req: HttpJsonRequest<'_>) -> Result<HttpJsonResponse, HostError> {
        let url: reqwest::Url = req.url.parse().map_err(|_| HostError::Policy)?;
        let host = url.host_str().unwrap_or_default();
        if !req.allowed_hosts.contains(&host) {
            return Err(HostError::Policy);
        }
        if url.scheme() != "https" {
            #[cfg(not(any(test, debug_assertions)))]
            return Err(HostError::Policy);
            #[cfg(any(test, debug_assertions))]
            if url.scheme() != "http" || !matches!(host, "127.0.0.1" | "localhost") {
                return Err(HostError::Policy);
            }
        }
        let default_limit = if self.max_bytes_default > 0 {
            self.max_bytes_default
        } else {
            super::policy::MAX_HTTP_BYTES
        };
        let limit = if req.max_bytes > 0 {
            req.max_bytes.min(default_limit)
        } else {
            default_limit
        };

        let client = self.client().await?;
        let mut builder = client.get(url);
        if let Some(secret) = req.bearer {
            let token = String::from_utf8(secret.to_vec()).map_err(|_| HostError::Policy)?;
            builder = builder.bearer_auth(token);
        }
        builder = builder.header(reqwest::header::USER_AGENT, req.user_agent);

        let deadline = tokio::time::Instant::now() + req.timeout;

        // Cancel-safe: dropping the future drops the in-flight request.
        let send = async {
            let response = tokio::time::timeout_at(deadline, builder.send())
                .await
                .map_err(|_| HostError::Timeout)?
                .map_err(|e| {
                    if e.is_timeout() {
                        HostError::Timeout
                    } else {
                        HostError::Unavailable
                    }
                })?;
            Ok::<_, HostError>(response)
        };
        let mut response = tokio::select! {
            biased;
            _ = req.cancel.cancelled() => return Err(HostError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => return Err(HostError::Timeout),
            result = send => result?,
        };

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

        let mut bytes = Vec::with_capacity(1024.min(limit));
        loop {
            let chunk_res = tokio::select! {
                biased;
                _ = req.cancel.cancelled() => return Err(HostError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => return Err(HostError::Timeout),
                res = response.chunk() => res,
            };
            match chunk_res {
                Ok(Some(chunk)) => {
                    if bytes.len().saturating_add(chunk.len()) > limit {
                        return Err(HostError::Policy);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    if e.is_timeout() {
                        return Err(HostError::Timeout);
                    } else {
                        return Err(HostError::Unavailable);
                    }
                }
            }
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

/// Extract `Retry-After` delay in seconds without logging the value.
/// Supports delta-seconds and the IMF-fixdate HTTP-date form; past
/// dates saturate to zero and absurd values are capped.
pub fn parse_retry_after(value: Option<&str>) -> Option<u64> {
    let raw = value?.trim();
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(secs.min(3600));
    }
    let at: std::time::SystemTime = httpdate::parse_http_date(raw).ok()?;
    let delta = at
        .duration_since(std::time::SystemTime::now())
        .unwrap_or_default();
    Some(delta.as_secs().min(3600))
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
                cancel: CancellationToken::new(),
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
                cancel: CancellationToken::new(),
            })
            .await;
        assert!(matches!(result, Err(HostError::Policy)));
    }

    #[tokio::test]
    async fn cancelled_request_finishes_without_transport() {
        let host = ReqwestHttpHost::default();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = host
            .get_json(HttpJsonRequest {
                url: "https://api.openai.com/v1/x",
                bearer: None,
                allowed_hosts: &["api.openai.com"],
                timeout: Duration::from_secs(5),
                max_bytes: 1024,
                user_agent: "Usage.ai-test",
                cancel,
            })
            .await;
        assert!(matches!(result, Err(HostError::Cancelled)));
    }

    #[test]
    fn retry_after_parses_digits_and_http_date() {
        assert_eq!(parse_retry_after(Some("77")), Some(77));
        assert_eq!(parse_retry_after(Some("soon")), None);
        assert_eq!(parse_retry_after(None), None);
        // Far-future HTTP-date is capped; past dates saturate to zero.
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2099 07:28:00 GMT")),
            Some(3600)
        );
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
            Some(0)
        );
    }

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn spawn_loopback_server<F, Fut>(handler: F) -> (String, tokio::task::JoinHandle<()>)
    where
        F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/test");
        let handle = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                handler(stream).await;
            }
        });
        (url, handle)
    }

    #[tokio::test]
    async fn payload_512_bytes_succeeds() {
        let (url, server) = spawn_loopback_server(|mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let prefix = r#"{"data":""#;
            let suffix = r#""}"#;
            let target_len = 512;
            let fill_len = target_len - prefix.len() - suffix.len();
            let body = format!("{prefix}{}{suffix}", "x".repeat(fill_len));
            assert_eq!(body.len(), 512);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 512\r\nConnection: close\r\n\r\n{body}"
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: 0,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await
            .expect("512-byte payload must succeed");
        assert_eq!(result.status, 200);
        let _ = server.await;
    }

    #[tokio::test]
    async fn large_4k_payload_succeeds_with_default_limits() {
        let (url, server) = spawn_loopback_server(|mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let large_str = "a".repeat(4000);
            let body = format!(r#"{{"key":"{large_str}"}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: 0,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await
            .expect("4 KiB payload must succeed under default 2 MiB limit");

        assert_eq!(result.status, 200);
        let val = result.body.unwrap();
        assert_eq!(val["key"].as_str().unwrap().len(), 4000);
        let _ = server.await;
    }

    #[tokio::test]
    async fn payload_approx_1_mib_succeeds() {
        let (url, server) = spawn_loopback_server(|mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let large_str = "a".repeat(1_000_000);
            let body = format!(r#"{{"key":"{large_str}"}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: 0,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await
            .expect("~1 MiB payload must succeed under default 2 MiB limit");

        assert_eq!(result.status, 200);
        let val = result.body.unwrap();
        assert_eq!(val["key"].as_str().unwrap().len(), 1_000_000);
        let _ = server.await;
    }

    #[tokio::test]
    async fn payload_exactly_maximum_succeeds() {
        let max_limit = 20_000;
        let (url, server) = spawn_loopback_server(move |mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let prefix = r#"{"k":""#;
            let suffix = r#""}"#;
            let pad = "z".repeat(max_limit - prefix.len() - suffix.len());
            let body = format!("{prefix}{pad}{suffix}");
            assert_eq!(body.len(), max_limit);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {max_limit}\r\nConnection: close\r\n\r\n{body}"
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: max_limit,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await
            .expect("Payload of exactly max_bytes must succeed");

        assert_eq!(result.status, 200);
        let _ = server.await;
    }

    #[tokio::test]
    async fn payload_maximum_plus_one_byte_fails_policy() {
        let max_limit = 20_000;
        let (url, server) = spawn_loopback_server(move |mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let prefix = r#"{"k":""#;
            let suffix = r#""}"#;
            let pad = "z".repeat(max_limit + 1 - prefix.len() - suffix.len());
            let body = format!("{prefix}{pad}{suffix}");
            assert_eq!(body.len(), max_limit + 1);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                max_limit + 1
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: max_limit,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await;

        assert!(matches!(result, Err(HostError::Policy)));
        let _ = server.await;
    }

    #[tokio::test]
    async fn chunked_payload_greater_than_1k_succeeds_up_to_max() {
        let (url, server) = spawn_loopback_server(|mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(headers.as_bytes()).await;
            let part1 = b"{\"text\":\"";
            let _ = stream.write_all(format!("{:x}\r\n", part1.len()).as_bytes()).await;
            let _ = stream.write_all(part1).await;
            let _ = stream.write_all(b"\r\n").await;

            let chunk_fill = "c".repeat(4096);
            for _ in 0..4 {
                let _ = stream.write_all(format!("{:x}\r\n", chunk_fill.len()).as_bytes()).await;
                let _ = stream.write_all(chunk_fill.as_bytes()).await;
                let _ = stream.write_all(b"\r\n").await;
            }

            let part_end = b"\"}";
            let _ = stream.write_all(format!("{:x}\r\n", part_end.len()).as_bytes()).await;
            let _ = stream.write_all(part_end).await;
            let _ = stream.write_all(b"\r\n").await;

            let _ = stream.write_all(b"0\r\n\r\n").await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: 0,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await
            .expect("Chunked payload > 1 KiB must succeed under default 2 MiB limit");

        assert_eq!(result.status, 200);
        let val = result.body.unwrap();
        assert_eq!(val["text"].as_str().unwrap().len(), 4096 * 4);
        let _ = server.await;
    }

    #[tokio::test]
    async fn fake_content_length_smaller_than_body_enforces_actual_max() {
        let (url, server) = spawn_loopback_server(|mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            // Fake Content-Length claims 50 bytes, but Transfer-Encoding is chunked and streams > 2000 bytes
            let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 50\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(headers.as_bytes()).await;
            let chunk_data = "e".repeat(1500);
            let hex_len = format!("{:x}\r\n", chunk_data.len());
            for _ in 0..2 {
                let _ = stream.write_all(hex_len.as_bytes()).await;
                let _ = stream.write_all(chunk_data.as_bytes()).await;
                let _ = stream.write_all(b"\r\n").await;
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: 2000,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await;

        assert!(
            matches!(result, Err(HostError::Policy)),
            "Must reject when streamed chunks exceed max_bytes despite fake Content-Length"
        );
        let _ = server.await;
    }

    #[tokio::test]
    async fn payload_exceeding_max_bytes_fails_policy() {
        let (url, server) = spawn_loopback_server(|mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let large_str = "b".repeat(3000);
            let body = format!(r#"{{"key":"{large_str}"}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: 2048,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await;

        assert!(matches!(result, Err(HostError::Policy)));
        let _ = server.await;
    }

    #[tokio::test]
    async fn chunked_body_without_content_length_caught_during_streaming() {
        let (url, server) = spawn_loopback_server(|mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(headers.as_bytes()).await;
            let chunk_data = "a".repeat(800);
            let hex_len = format!("{:x}\r\n", chunk_data.len());
            for _ in 0..3 {
                let _ = stream.write_all(hex_len.as_bytes()).await;
                let _ = stream.write_all(chunk_data.as_bytes()).await;
                let _ = stream.write_all(b"\r\n").await;
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: 1000,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await;

        assert!(matches!(result, Err(HostError::Policy)));
        let _ = server.await;
    }

    #[tokio::test]
    async fn cancellation_during_body_streaming_cancels_promptly() {
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let (url, server) = spawn_loopback_server(move |mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = stream.write_all(headers.as_bytes()).await;
            let _ = stream.write_all(b"5\r\n{\"a\":\r\n").await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_clone.cancel();
            tokio::time::sleep(Duration::from_millis(500)).await;
        }).await;

        let host = ReqwestHttpHost::default();
        let start = std::time::Instant::now();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_secs(5),
                max_bytes: 1024 * 1024,
                user_agent: "Usage.ai-test",
                cancel,
            })
            .await;

        assert!(matches!(result, Err(HostError::Cancelled)));
        assert!(start.elapsed() < Duration::from_secs(2));
        let _ = server.await;
    }

    #[tokio::test]
    async fn timeout_during_body_streaming_times_out() {
        let (url, server) = spawn_loopback_server(move |mut stream| async move {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n";
            let _ = stream.write_all(headers.as_bytes()).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }).await;

        let host = ReqwestHttpHost::default();
        let result = host
            .get_json(HttpJsonRequest {
                url: &url,
                bearer: None,
                allowed_hosts: &["127.0.0.1"],
                timeout: Duration::from_millis(100),
                max_bytes: 1024 * 1024,
                user_agent: "Usage.ai-test",
                cancel: CancellationToken::new(),
            })
            .await;

        assert!(matches!(result, Err(HostError::Timeout)));
        let _ = server.await;
    }
}
