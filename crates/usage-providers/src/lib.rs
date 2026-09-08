pub mod claude;
pub mod codex;
pub mod discovery;
pub mod http;
pub mod openai_api;

use sha2::{Digest, Sha256};
use std::path::Path;

pub fn anonymous_path_identity(path: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(path.as_os_str().to_string_lossy().as_bytes());
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod fixture_security {
    #[test]
    fn fixtures_do_not_contain_forbidden_fields() {
        for fixture in [
            include_str!("../fixtures/codex-token-count.jsonl"),
            include_str!("../fixtures/claude-usage.jsonl"),
        ] {
            let lower = fixture.to_ascii_lowercase();
            for forbidden in [
                "authorization",
                "access_token",
                "refresh_token",
                "api_key",
                "cookie",
                "password",
                "\"prompt\"",
                "\"content\"",
                "email",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "fixture contains forbidden field: {forbidden}"
                );
            }
        }
    }
}
