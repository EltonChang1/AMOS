use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::{Result, domain::Identity, error::AmosError, seed::TENANT};

pub trait IdentityProvider: Send + Sync {
    fn authenticate_bearer(&self, bearer_token: &str) -> Result<Identity>;
}

#[derive(Clone)]
pub struct StaticIdentityProvider {
    identities: Arc<BTreeMap<String, Identity>>,
}

impl StaticIdentityProvider {
    pub fn new(identities: BTreeMap<String, Identity>) -> Self {
        Self {
            identities: Arc::new(identities),
        }
    }

    pub fn demo() -> Self {
        Self::new(demo_identities())
    }
}

impl IdentityProvider for StaticIdentityProvider {
    fn authenticate_bearer(&self, bearer_token: &str) -> Result<Identity> {
        self.identities
            .get(bearer_token)
            .cloned()
            .ok_or_else(|| AmosError::Unauthenticated("invalid bearer credentials".into()))
    }
}

pub fn demo_identities() -> BTreeMap<String, Identity> {
    let identity = |subject: &str, roles: &[&str], permissions: &[&str]| Identity {
        tenant_id: TENANT.into(),
        subject_id: subject.into(),
        roles: roles.iter().map(|value| value.to_string()).collect(),
        groups: BTreeSet::new(),
        permissions: permissions.iter().map(|value| value.to_string()).collect(),
        policy_attributes: BTreeMap::new(),
        policy_epoch: 1,
    };

    BTreeMap::from([
        (
            "analyst_001".into(),
            identity("analyst_001", &["analyst"], &["analytics", "payments"]),
        ),
        (
            "analyst_002".into(),
            identity("analyst_002", &["analyst"], &["analytics", "payments"]),
        ),
        (
            "reviewer_001".into(),
            identity("reviewer_001", &["reviewer"], &["analytics", "payments"]),
        ),
        (
            "admin".into(),
            identity(
                "admin",
                &["admin", "owner", "reviewer"],
                &["analytics", "payments", "sre", "admin"],
            ),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_provider_rejects_unknown_credentials_as_unauthenticated() {
        let provider = StaticIdentityProvider::demo();

        assert!(matches!(
            provider.authenticate_bearer("unknown"),
            Err(AmosError::Unauthenticated(_))
        ));
    }
}
