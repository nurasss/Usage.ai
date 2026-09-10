pub mod claude;
pub mod codex;
pub mod codex_appserver;
pub mod descriptor;
pub mod discovery;
pub mod openai_api;
pub mod strategy;

use sha2::{Digest, Sha256};
use std::path::Path;

pub fn anonymous_path_identity(path: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(path.as_os_str().to_string_lossy().as_bytes());
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod host_boundary {
    /// Tripwire: production provider code must not touch the OS
    /// directly. Filesystem, environment and network go through
    /// `usage-host`; `std::fs`/`glob`/`env`/`reqwest` tokens are only
    /// allowed inside `#[cfg(test)]` modules (fixture loading) with an
    /// explicit justification comment on this test.
    #[test]
    fn production_code_has_no_direct_os_access() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let banned = ["std::fs", "tokio::fs", "glob::", "std::env", "reqwest::"];
        let mut violations = vec![];
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let content = std::fs::read_to_string(&path).unwrap();
            // Only scan production code: everything from the first
            // `#[cfg(test)]` to EOF is test utility.
            let production = content.split("#[cfg(test)]").next().unwrap_or("");
            for token in banned {
                if production.contains(token) {
                    violations.push(format!(
                        "{}: forbidden `{token}` in production code",
                        path.display()
                    ));
                }
            }
        }
        assert!(violations.is_empty(), "{violations:#?}");
    }
}
#[cfg(test)]
mod fixture_security {
    #[test]
    fn fixtures_do_not_contain_forbidden_fields() {
        // Every synthetic fixture file, old and new: structure and
        // usage/rate fields only — no prompts, responses, secrets,
        // identities or paths. These files are not real-client evidence.
        let corpus = [
            include_str!("../fixtures/codex-token-count.jsonl"),
            include_str!("../fixtures/claude-usage.jsonl"),
            include_str!("../fixtures/codex/session-a.jsonl"),
            include_str!("../fixtures/codex/session-b.jsonl"),
            include_str!("../fixtures/codex/duplicates.jsonl"),
            include_str!("../fixtures/codex/malformed.jsonl"),
            include_str!("../fixtures/codex/archived/old.jsonl"),
            include_str!("../fixtures/claude/thread-a.jsonl"),
            include_str!("../fixtures/claude/thread-b.jsonl"),
            include_str!("../fixtures/real/codex-session.jsonl"),
            include_str!("../fixtures/real/codex-controlled-1.jsonl"),
            include_str!("../fixtures/real/claude-thread.jsonl"),
        ];
        for fixture in corpus {
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
                "/users/",
                "/home/",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "fixture contains forbidden field: {forbidden}"
                );
            }
        }
    }
}
