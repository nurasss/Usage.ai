/// Whitelist structured logger. Implementations must never log
/// bearer tokens, cookies, API keys, prompts, responses, raw
/// Authorization headers, raw paths or unfiltered provider bodies.
pub trait LoggerHost: Send + Sync {
    fn info(&self, event: &str, fields: &[(&str, &str)]);
    fn warn(&self, event: &str, fields: &[(&str, &str)]);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TracingLogger;

impl LoggerHost for TracingLogger {
    fn info(&self, event: &str, fields: &[(&str, &str)]) {
        tracing::info!(event, fields = ?Redacted(fields));
    }
    fn warn(&self, event: &str, fields: &[(&str, &str)]) {
        tracing::warn!(event, fields = ?Redacted(fields));
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NullLogger;

impl LoggerHost for NullLogger {
    fn info(&self, _event: &str, _fields: &[(&str, &str)]) {}
    fn warn(&self, _event: &str, _fields: &[(&str, &str)]) {}
}

struct Redacted<'a>(&'a [(&'a str, &'a str)]);

impl std::fmt::Debug for Redacted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let safe: Vec<(&str, &str)> = self
            .0
            .iter()
            .map(|(k, v)| (*k, crate::logger::redact_value(k, v)))
            .collect();
        f.debug_list().entries(safe.iter()).finish()
    }
}

const SENSITIVE_KEYS: &[&str] = &[
    "authorization",
    "bearer",
    "token",
    "secret",
    "password",
    "cookie",
    "api_key",
    "apikey",
    "path",
    "prompt",
    "response",
    "body",
];

pub fn redact_value<'a>(key: &str, value: &'a str) -> &'a str {
    let lower = key.to_ascii_lowercase();
    if SENSITIVE_KEYS.iter().any(|needle| lower.contains(needle)) {
        "[REDACTED]"
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sensitive_keys_are_redacted() {
        assert_eq!(redact_value("authorization", "Bearer x"), "[REDACTED]");
        assert_eq!(redact_value("path", "/Users/x"), "[REDACTED]");
        assert_eq!(redact_value("pool_id", "abc"), "abc");
    }
}
