use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashedIdentityManifest {
    pub schema_version: u32,
    pub identities: Vec<HashedIdentityEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashedIdentityEntry {
    pub token_sha256: String,
    pub identity: Identity,
}

#[derive(Clone)]
pub struct HashedTokenIdentityProvider {
    identities: Arc<BTreeMap<String, Identity>>,
}

impl HashedTokenIdentityProvider {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let body = fs::read(path.as_ref()).map_err(|error| {
            AmosError::Storage(format!(
                "failed to read identity manifest {}: {error}",
                path.as_ref().display()
            ))
        })?;
        let manifest: HashedIdentityManifest = serde_json::from_slice(&body)?;
        Self::from_manifest(manifest)
    }

    pub fn from_manifest(manifest: HashedIdentityManifest) -> Result<Self> {
        if manifest.schema_version != 1 {
            return Err(AmosError::Validation(format!(
                "unsupported identity manifest schema version {}; expected 1",
                manifest.schema_version
            )));
        }
        if manifest.identities.is_empty() {
            return Err(AmosError::Validation(
                "identity manifest must contain at least one identity".into(),
            ));
        }
        let mut identities = BTreeMap::new();
        let mut subjects = BTreeSet::new();
        for entry in manifest.identities {
            validate_token_hash(&entry.token_sha256)?;
            validate_identity(&entry.identity)?;
            if !subjects.insert((
                entry.identity.tenant_id.clone(),
                entry.identity.subject_id.clone(),
            )) {
                return Err(AmosError::Validation(format!(
                    "identity manifest repeats subject {} in tenant {}",
                    entry.identity.subject_id, entry.identity.tenant_id
                )));
            }
            if identities
                .insert(entry.token_sha256, entry.identity)
                .is_some()
            {
                return Err(AmosError::Validation(
                    "identity manifest contains a duplicate token hash".into(),
                ));
            }
        }
        Ok(Self {
            identities: Arc::new(identities),
        })
    }

    pub fn token_hash(token: &str) -> Result<String> {
        if token.len() < 32 || token.len() > 4096 || token.chars().any(char::is_whitespace) {
            return Err(AmosError::Validation(
                "bearer token must contain 32 to 4096 non-whitespace characters".into(),
            ));
        }
        Ok(hex::encode(Sha256::digest(token.as_bytes())))
    }
}

impl IdentityProvider for HashedTokenIdentityProvider {
    fn authenticate_bearer(&self, bearer_token: &str) -> Result<Identity> {
        let token_hash = Self::token_hash(bearer_token)
            .map_err(|_| AmosError::Unauthenticated("invalid bearer credentials".into()))?;
        self.identities
            .get(&token_hash)
            .cloned()
            .ok_or_else(|| AmosError::Unauthenticated("invalid bearer credentials".into()))
    }
}

fn validate_token_hash(token_hash: &str) -> Result<()> {
    if token_hash.len() != 64
        || token_hash
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(AmosError::Validation(
            "identity token_sha256 must be 64 lowercase hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn validate_identity(identity: &Identity) -> Result<()> {
    if identity.tenant_id.trim().is_empty()
        || identity.subject_id.trim().is_empty()
        || identity.roles.is_empty()
        || identity.policy_epoch == 0
    {
        return Err(AmosError::Validation(
            "identity requires tenant_id, subject_id, at least one role, and a positive policy_epoch"
                .into(),
        ));
    }
    Ok(())
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

    #[test]
    fn hashed_provider_authenticates_without_storing_plaintext_tokens() {
        let token = "customer-evaluation-token-with-enough-entropy";
        let identity = demo_identities()["analyst_001"].clone();
        let provider = HashedTokenIdentityProvider::from_manifest(HashedIdentityManifest {
            schema_version: 1,
            identities: vec![HashedIdentityEntry {
                token_sha256: HashedTokenIdentityProvider::token_hash(token).unwrap(),
                identity: identity.clone(),
            }],
        })
        .unwrap();

        assert_eq!(provider.authenticate_bearer(token).unwrap(), identity);
        assert!(matches!(
            provider.authenticate_bearer("another-customer-token-with-enough-entropy"),
            Err(AmosError::Unauthenticated(_))
        ));
        assert!(!format!("{:?}", provider.identities).contains(token));
    }

    #[test]
    fn hashed_provider_rejects_invalid_manifests() {
        let identity = demo_identities()["analyst_001"].clone();
        assert!(matches!(
            HashedTokenIdentityProvider::from_manifest(HashedIdentityManifest {
                schema_version: 2,
                identities: vec![]
            }),
            Err(AmosError::Validation(_))
        ));
        assert!(matches!(
            HashedTokenIdentityProvider::from_manifest(HashedIdentityManifest {
                schema_version: 1,
                identities: vec![HashedIdentityEntry {
                    token_sha256: "not-a-hash".into(),
                    identity,
                }]
            }),
            Err(AmosError::Validation(_))
        ));
    }
}
