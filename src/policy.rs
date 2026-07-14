use std::collections::BTreeSet;

use crate::{
    Result,
    domain::{Authority, Identity, MemoryObject, RiskClass, TaskDefinition},
    error::AmosError,
};

#[derive(Debug, Clone, Default)]
pub struct PolicyEngine;

impl PolicyEngine {
    pub fn can_read_memory(&self, identity: &Identity, object: &MemoryObject) -> bool {
        identity.tenant_id == object.tenant_id
            && object.permissions.is_subset(&identity.permissions)
            && !matches!(
                object.status,
                crate::domain::MemoryStatus::Revoked | crate::domain::MemoryStatus::Tombstoned
            )
    }

    pub fn authorize_memory_write(&self, identity: &Identity, object: &MemoryObject) -> Result<()> {
        if identity.tenant_id != object.tenant_id {
            return Err(AmosError::PermissionDenied(
                "cross-tenant memory write".into(),
            ));
        }
        let allowed = match object.authority {
            Authority::OwnerApproved => has_any(&identity.roles, &["owner", "admin"]),
            Authority::ReviewerApproved => {
                has_any(&identity.roles, &["reviewer", "owner", "admin"])
            }
            Authority::SystemObserved => {
                has_any(&identity.roles, &["connector", "system", "admin"])
            }
            Authority::UserNote => true,
            Authority::ModelHypothesis => has_any(&identity.roles, &["runtime", "system", "admin"]),
            Authority::UntrustedExternal => {
                has_any(&identity.roles, &["connector", "system", "admin"])
            }
        };
        if !allowed {
            return Err(AmosError::PermissionDenied(format!(
                "identity cannot write {:?} memory",
                object.authority
            )));
        }
        Ok(())
    }

    pub fn authorize_task(&self, identity: &Identity, definition: &TaskDefinition) -> Result<()> {
        if identity.tenant_id != definition.tenant_id {
            return Err(AmosError::PermissionDenied("cross-tenant task".into()));
        }
        if !definition.status.eq_ignore_ascii_case("approved") {
            return Err(AmosError::Validation(
                "task definition is not approved".into(),
            ));
        }
        if matches!(
            definition.risk_class,
            RiskClass::External | RiskClass::Regulated
        ) && !has_any(&identity.roles, &["reviewer", "owner", "admin"])
        {
            return Err(AmosError::PermissionDenied(
                "high-risk task requires reviewer role".into(),
            ));
        }
        Ok(())
    }

    pub fn authorize_tool(
        &self,
        identity: &Identity,
        definition: &TaskDefinition,
        tool: &str,
        relations: &BTreeSet<String>,
    ) -> Result<()> {
        if !definition.allowed_tools.contains(tool) {
            return Err(AmosError::PermissionDenied(format!(
                "tool {tool} is not allowed"
            )));
        }
        if !relations.is_subset(&identity.permissions) {
            return Err(AmosError::PermissionDenied(
                "tool relation is not permitted".into(),
            ));
        }
        Ok(())
    }

    pub fn authorize_review(&self, identity: &Identity, owner_authority: bool) -> Result<()> {
        let allowed = if owner_authority {
            has_any(&identity.roles, &["owner", "admin"])
        } else {
            has_any(&identity.roles, &["reviewer", "owner", "admin"])
        };
        if allowed {
            Ok(())
        } else {
            Err(AmosError::PermissionDenied(
                "reviewer authority required".into(),
            ))
        }
    }
}

fn has_any(values: &BTreeSet<String>, expected: &[&str]) -> bool {
    expected.iter().any(|value| values.contains(*value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{MemoryObject, MemoryType};
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn identity() -> Identity {
        Identity {
            tenant_id: "t".into(),
            subject_id: "u".into(),
            roles: BTreeSet::from(["analyst".into()]),
            groups: BTreeSet::new(),
            permissions: BTreeSet::from(["payments".into()]),
            policy_attributes: BTreeMap::new(),
            policy_epoch: 1,
        }
    }

    #[test]
    fn permission_filter_is_all_labels() {
        let mut object = MemoryObject::new(
            "t",
            "schema",
            MemoryType::Schema,
            "schema",
            json!({}),
            "catalog",
            "1",
            Authority::OwnerApproved,
        );
        object.permissions = BTreeSet::from(["payments".into(), "sre".into()]);
        assert!(!PolicyEngine.can_read_memory(&identity(), &object));
    }

    #[test]
    fn analyst_cannot_forge_owner_memory() {
        let object = MemoryObject::new(
            "t",
            "metric",
            MemoryType::SemanticDefinition,
            "metric",
            json!({}),
            "user",
            "1",
            Authority::OwnerApproved,
        );
        assert!(
            PolicyEngine
                .authorize_memory_write(&identity(), &object)
                .is_err()
        );
    }
}
