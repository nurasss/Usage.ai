use super::{HostError, SecretString};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type SecretMap = HashMap<(String, String), Vec<u8>>;

/// Typed Keychain access. Callers address concrete (service, account)
/// pairs; there is no scan/list operation by design.
#[async_trait]
pub trait KeychainHost: Send + Sync {
    async fn read(&self, service: &str, account: &str) -> Result<SecretString, HostError>;
    async fn write(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), HostError>;
    async fn delete(&self, service: &str, account: &str) -> Result<(), HostError>;
}

/// macOS native Keychain via the `keyring` crate.
#[cfg(target_os = "macos")]
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringHost;

#[cfg(target_os = "macos")]
#[async_trait]
impl KeychainHost for KeyringHost {
    async fn read(&self, service: &str, account: &str) -> Result<SecretString, HostError> {
        let entry = keyring::Entry::new(service, account).map_err(|_| HostError::Unavailable)?;
        let password = entry.get_password().map_err(|_| HostError::Denied)?;
        if password.is_empty() {
            return Err(HostError::NotFound);
        }
        Ok(SecretString::new(password.into_bytes()))
    }
    async fn write(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), HostError> {
        let text = String::from_utf8(secret.to_vec()).map_err(|_| HostError::Policy)?;
        if text.is_empty() || text.len() > 4096 {
            return Err(HostError::Policy);
        }
        let entry = keyring::Entry::new(service, account).map_err(|_| HostError::Unavailable)?;
        entry.set_password(&text).map_err(|_| HostError::Denied)?;
        Ok(())
    }
    async fn delete(&self, service: &str, account: &str) -> Result<(), HostError> {
        let entry = keyring::Entry::new(service, account).map_err(|_| HostError::Unavailable)?;
        entry.delete_credential().map_err(|_| HostError::Denied)?;
        Ok(())
    }
}

/// In-memory Keychain for tests and non-Keychain platforms.
/// Never leaves the process; holds no real credentials in production.
#[derive(Debug, Default, Clone)]
pub struct MemoryKeychain {
    inner: Arc<Mutex<SecretMap>>,
}

impl MemoryKeychain {
    pub fn with_secret(self, service: &str, account: &str, secret: &[u8]) -> Self {
        if let Ok(mut map) = self.inner.lock() {
            map.insert((service.into(), account.into()), secret.to_vec());
        }
        self
    }
}

#[async_trait]
impl KeychainHost for MemoryKeychain {
    async fn read(&self, service: &str, account: &str) -> Result<SecretString, HostError> {
        self.inner
            .lock()
            .map_err(|_| HostError::Unavailable)?
            .get(&(service.to_string(), account.to_string()))
            .cloned()
            .filter(|v| !v.is_empty())
            .map(SecretString::new)
            .ok_or(HostError::NotFound)
    }
    async fn write(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), HostError> {
        if secret.is_empty() || secret.len() > 4096 {
            return Err(HostError::Policy);
        }
        self.inner
            .lock()
            .map_err(|_| HostError::Unavailable)?
            .insert((service.into(), account.into()), secret.to_vec());
        Ok(())
    }
    async fn delete(&self, service: &str, account: &str) -> Result<(), HostError> {
        self.inner
            .lock()
            .map_err(|_| HostError::Unavailable)?
            .remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

/// Namespace-scoped Keychain facade. The service is fixed at
/// construction by the runtime from the product descriptor, so a
/// provider can only address accounts inside its own namespace —
/// never an arbitrary (service, account) pair.
#[derive(Debug, Clone, Copy)]
pub struct ScopedKeychain<'a, H: KeychainHost + ?Sized> {
    inner: &'a H,
    service: &'a str,
}

impl<'a, H: KeychainHost + ?Sized> ScopedKeychain<'a, H> {
    pub fn new(inner: &'a H, service: &'a str) -> Self {
        Self { inner, service }
    }

    pub fn service(&self) -> &str {
        self.service
    }

    fn check_account(account: &str) -> Result<(), HostError> {
        if account.is_empty() || account.len() > 128 {
            return Err(HostError::Policy);
        }
        Ok(())
    }

    pub async fn read(&self, account: &str) -> Result<SecretString, HostError> {
        Self::check_account(account)?;
        self.inner.read(self.service, account).await
    }

    pub async fn write(&self, account: &str, secret: &[u8]) -> Result<(), HostError> {
        Self::check_account(account)?;
        self.inner.write(self.service, account, secret).await
    }

    pub async fn delete(&self, account: &str) -> Result<(), HostError> {
        Self::check_account(account)?;
        self.inner.delete(self.service, account).await
    }
}

#[cfg(test)]
mod namespace_tests {
    use super::*;
    #[tokio::test]
    async fn scopes_are_isolated_by_service() {
        let inner = MemoryKeychain::default();
        let a = ScopedKeychain::new(&inner, "service-a");
        let b = ScopedKeychain::new(&inner, "service-b");
        a.write("item", b"secret-a").await.unwrap();
        assert!(b.read("item").await.is_err());
        assert_eq!(a.read("item").await.unwrap().expose(), b"secret-a");
    }
    #[tokio::test]
    async fn empty_or_oversized_accounts_rejected() {
        let inner = MemoryKeychain::default();
        let scope = ScopedKeychain::new(&inner, "service-a");
        assert!(scope.read("").await.is_err());
        assert!(scope.read(&"x".repeat(129)).await.is_err());
    }
}
