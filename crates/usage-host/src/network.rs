use std::sync::{Arc, Mutex};

/// Observed connectivity. `Unknown` means no observation yet —
/// local-only sources keep working regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnlineState {
    Online,
    Offline,
    #[default]
    Unknown,
}

pub trait NetworkHost: Send + Sync {
    fn state(&self) -> OnlineState;
}

/// Process-wide observed network state, fed by the platform layer
/// (frontend online/offline events). Never hardcoded per product.
#[derive(Debug, Default, Clone)]
pub struct ObservedNetwork {
    inner: Arc<Mutex<OnlineState>>,
}

impl ObservedNetwork {
    pub fn set(&self, state: OnlineState) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = state;
        }
    }
}

impl NetworkHost for ObservedNetwork {
    fn state(&self) -> OnlineState {
        self.inner
            .lock()
            .map(|g| *g)
            .unwrap_or(OnlineState::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_to_unknown_then_updates() {
        let net = ObservedNetwork::default();
        assert_eq!(net.state(), OnlineState::Unknown);
        net.set(OnlineState::Offline);
        assert_eq!(net.state(), OnlineState::Offline);
    }
}
