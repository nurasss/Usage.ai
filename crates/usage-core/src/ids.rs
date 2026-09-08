use serde::{Deserialize, Serialize};
use std::fmt;

/// Typed identifiers. Newtypes prevent accidental mixing of
/// provider / product / account / source / scope namespaces.
macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

string_id!(ProviderId);
string_id!(ProductId);
string_id!(SourceId);
string_id!(BillingScopeId);
string_id!(PoolId);
string_id!(WindowId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub uuid::Uuid);

impl AccountId {
    pub fn new(id: uuid::Uuid) -> Self {
        Self(id)
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<uuid::Uuid> for AccountId {
    fn from(id: uuid::Uuid) -> Self {
        Self(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespaces_do_not_convert_implicitly() {
        fn takes_product(_: ProductId) {}
        let provider = ProviderId::new("openai");
        // This would fail to compile if uncommented:
        // takes_product(provider);
        takes_product(ProductId::new(provider.as_str()));
    }
}
