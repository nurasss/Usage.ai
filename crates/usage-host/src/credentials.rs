use sha2::{Digest, Sha256};

/// In-memory secret that is zeroed on drop. It is never Display/Debug
/// printable and never serialized.
pub struct SecretString {
    inner: Vec<u8>,
}

impl SecretString {
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        Self {
            inner: value.into(),
        }
    }

    pub fn expose(&self) -> &[u8] {
        &self.inner
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        for byte in self.inner.iter_mut() {
            // Prevent dead-store elimination of the wipe.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}

/// Stable non-privileged credential fingerprint: `provider:hex16`.
/// Never reversible; safe for identity, diagnostics and dedup keys.
pub fn fingerprint(provider: &str, secret: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(provider.as_bytes());
    digest.update([0u8]);
    digest.update(secret);
    format!("{provider}:{}", &format!("{:x}", digest.finalize())[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprint_is_stable_scoped_and_opaque() {
        let a = fingerprint("openai-api", b"sk-test-1");
        assert_eq!(a, fingerprint("openai-api", b"sk-test-1"));
        assert!(a.starts_with("openai-api:"));
        assert!(!a.contains("sk-test-1"));
        assert_ne!(a, fingerprint("other", b"sk-test-1"));
    }
}
