use std::{
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{
    Result,
    auth::{HashedTokenIdentityProvider, IdentityProvider},
    error::AmosError,
    runtime::RuntimeConfig,
};

pub const SERVER_CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentMode {
    CustomerEvaluation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub schema_version: u32,
    pub deployment_mode: DeploymentMode,
    pub acknowledge_local_reference_adapters: bool,
    pub bind_address: IpAddr,
    pub port: u16,
    pub public_base_url: String,
    pub control_db: PathBuf,
    pub warehouse_db: PathBuf,
    pub object_root: PathBuf,
    pub capability_key_file: PathBuf,
    pub identities_file: PathBuf,
    #[serde(default)]
    pub toolbox_endpoint: Option<String>,
}

pub struct LoadedServerConfig {
    pub server: ServerConfig,
    pub runtime: RuntimeConfig,
    pub identity_provider: Arc<dyn IdentityProvider>,
}

impl ServerConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let body = fs::read(path.as_ref()).map_err(|error| {
            AmosError::Storage(format!(
                "failed to read server configuration {}: {error}",
                path.as_ref().display()
            ))
        })?;
        let config: Self = serde_json::from_slice(&body)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SERVER_CONFIG_SCHEMA_VERSION {
            return Err(AmosError::Validation(format!(
                "unsupported server configuration schema version {}; expected {}",
                self.schema_version, SERVER_CONFIG_SCHEMA_VERSION
            )));
        }
        if !self.acknowledge_local_reference_adapters {
            return Err(AmosError::Validation(
                "customer-evaluation mode requires explicit acknowledgment that SQLite, static hashed bearer identities, the payment workflow, and local object storage are reference adapters, not production integrations".into(),
            ));
        }
        if self.port == 0 {
            return Err(AmosError::Validation(
                "server port must be greater than zero".into(),
            ));
        }
        if !self.public_base_url.starts_with("https://")
            || self.public_base_url.chars().any(char::is_whitespace)
        {
            return Err(AmosError::Validation(
                "public_base_url must be an absolute HTTPS URL without whitespace".into(),
            ));
        }
        let paths = [
            ("control_db", &self.control_db),
            ("warehouse_db", &self.warehouse_db),
            ("object_root", &self.object_root),
            ("capability_key_file", &self.capability_key_file),
            ("identities_file", &self.identities_file),
        ];
        for (name, path) in paths {
            if !path.is_absolute() {
                return Err(AmosError::Validation(format!(
                    "{name} must be an absolute path"
                )));
            }
        }
        if self.control_db == self.warehouse_db {
            return Err(AmosError::Validation(
                "control_db and warehouse_db must be different files".into(),
            ));
        }
        if self.capability_key_file == self.identities_file {
            return Err(AmosError::Validation(
                "capability_key_file and identities_file must be different files".into(),
            ));
        }
        if self.object_root == self.control_db || self.object_root == self.warehouse_db {
            return Err(AmosError::Validation(
                "object_root must be a directory distinct from database files".into(),
            ));
        }
        if self.toolbox_endpoint.as_deref().is_some_and(|endpoint| {
            endpoint.contains('/')
                || endpoint.chars().any(char::is_whitespace)
                || endpoint
                    .rsplit_once(':')
                    .is_none_or(|(host, port)| host.is_empty() || port.parse::<u16>().is_err())
        }) {
            return Err(AmosError::Validation(
                "toolbox_endpoint must use host:port without a URL path".into(),
            ));
        }
        Ok(())
    }

    pub fn load_runtime(&self) -> Result<LoadedServerConfig> {
        self.validate()?;
        let capability_key = read_capability_key(&self.capability_key_file)?;
        let identity_provider = HashedTokenIdentityProvider::load(&self.identities_file)?;
        let runtime = RuntimeConfig::new(
            self.control_db.clone(),
            self.warehouse_db.clone(),
            capability_key,
        )
        .with_object_root(self.object_root.clone());
        let runtime = match &self.toolbox_endpoint {
            Some(endpoint) => runtime.with_toolbox_endpoint(endpoint.clone()),
            None => runtime,
        };
        Ok(LoadedServerConfig {
            server: self.clone(),
            runtime,
            identity_provider: Arc::new(identity_provider),
        })
    }

    pub fn ensure_data_directories(&self) -> Result<()> {
        for path in [&self.control_db, &self.warehouse_db] {
            let parent = path.parent().ok_or_else(|| {
                AmosError::Validation(format!("{} has no parent directory", path.display()))
            })?;
            fs::create_dir_all(parent).map_err(|error| {
                AmosError::Storage(format!(
                    "failed to create data directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        fs::create_dir_all(&self.object_root).map_err(|error| {
            AmosError::Storage(format!(
                "failed to create object directory {}: {error}",
                self.object_root.display()
            ))
        })?;
        Ok(())
    }
}

pub fn read_capability_key(path: &Path) -> Result<Vec<u8>> {
    let encoded = fs::read_to_string(path).map_err(|error| {
        AmosError::Storage(format!(
            "failed to read capability key {}: {error}",
            path.display()
        ))
    })?;
    let encoded = encoded.trim();
    if encoded.len() != 64
        || encoded
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(AmosError::Validation(
            "capability key must contain exactly 32 bytes encoded as 64 lowercase hexadecimal characters"
                .into(),
        ));
    }
    hex::decode(encoded).map_err(|_| {
        AmosError::Validation("capability key contains invalid hexadecimal data".into())
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, net::Ipv4Addr};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        auth::{HashedIdentityEntry, HashedIdentityManifest, HashedTokenIdentityProvider},
        domain::Identity,
    };

    fn config(root: &Path) -> ServerConfig {
        ServerConfig {
            schema_version: 1,
            deployment_mode: DeploymentMode::CustomerEvaluation,
            acknowledge_local_reference_adapters: true,
            bind_address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: 8000,
            public_base_url: "https://amos.customer.example".into(),
            control_db: root.join("data/control.sqlite"),
            warehouse_db: root.join("data/warehouse.sqlite"),
            object_root: root.join("objects"),
            capability_key_file: root.join("secrets/capability-key"),
            identities_file: root.join("secrets/identities.json"),
            toolbox_endpoint: None,
        }
    }

    #[test]
    fn server_configuration_fails_closed_on_unsafe_or_ambiguous_values() {
        let root = TempDir::new().unwrap();
        let mut candidate = config(root.path());
        candidate.acknowledge_local_reference_adapters = false;
        assert!(matches!(
            candidate.validate(),
            Err(AmosError::Validation(_))
        ));

        let mut candidate = config(root.path());
        candidate.public_base_url = "http://amos.customer.example".into();
        assert!(matches!(
            candidate.validate(),
            Err(AmosError::Validation(_))
        ));

        let mut candidate = config(root.path());
        candidate.control_db = PathBuf::from("relative.sqlite");
        assert!(matches!(
            candidate.validate(),
            Err(AmosError::Validation(_))
        ));
    }

    #[test]
    fn server_configuration_loads_secrets_without_exposing_them_in_debug() {
        let root = TempDir::new().unwrap();
        let candidate = config(root.path());
        fs::create_dir_all(root.path().join("secrets")).unwrap();
        let capability_key = "11".repeat(32);
        fs::write(&candidate.capability_key_file, &capability_key).unwrap();
        let token = "customer-evaluation-token-with-enough-entropy";
        let manifest = HashedIdentityManifest {
            schema_version: 1,
            identities: vec![HashedIdentityEntry {
                token_sha256: HashedTokenIdentityProvider::token_hash(token).unwrap(),
                identity: Identity {
                    tenant_id: "tenant_demo".into(),
                    subject_id: "analyst".into(),
                    roles: BTreeSet::from(["analyst".into()]),
                    groups: BTreeSet::new(),
                    permissions: BTreeSet::from(["analytics".into(), "payments".into()]),
                    policy_attributes: Default::default(),
                    policy_epoch: 1,
                },
            }],
        };
        fs::write(
            &candidate.identities_file,
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let loaded = candidate.load_runtime().unwrap();
        let debug = format!("{:?}", loaded.runtime);
        assert!(!debug.contains(&capability_key));
        assert_eq!(loaded.runtime.object_root, candidate.object_root);
    }
}
