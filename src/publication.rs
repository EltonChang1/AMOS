use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use crate::{Result, domain::content_hash, error::AmosError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotedObject {
    pub key: String,
    pub path: PathBuf,
    pub content_hash: String,
    pub byte_count: u64,
}

pub trait ObjectStore: Send + Sync {
    fn stage(&self, key: &str, content: &str, expected_hash: &str) -> Result<PromotedObject>;
    fn promote(&self, key: &str, expected_hash: &str) -> Result<PromotedObject>;
    fn read(&self, key: &str) -> Result<Option<String>>;
}

#[derive(Debug, Clone)]
pub struct LocalFilesystemObjectStore {
    root: PathBuf,
}

impl LocalFilesystemObjectStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("staged"))?;
        fs::create_dir_all(root.join("objects"))?;
        Ok(Self { root })
    }

    fn path(&self, area: &str, key: &str) -> Result<PathBuf> {
        let path = Path::new(key);
        if path.is_absolute()
            || path.components().any(|component| {
                !matches!(component, Component::Normal(_))
                    || component.as_os_str().to_str().is_none_or(|part| {
                        part.is_empty()
                            || !part.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                            })
                    })
            })
        {
            return Err(AmosError::Validation(
                "object key contains an unsafe path component".into(),
            ));
        }
        Ok(self.root.join(area).join(path))
    }

    fn inspect(&self, key: &str, path: PathBuf, expected_hash: &str) -> Result<PromotedObject> {
        let content = fs::read_to_string(&path)?;
        let actual_hash = content_hash(&content)?;
        if actual_hash != expected_hash {
            return Err(AmosError::Conflict(format!(
                "object {key} does not match its expected content hash"
            )));
        }
        Ok(PromotedObject {
            key: key.into(),
            path,
            content_hash: actual_hash,
            byte_count: u64::try_from(content.len())
                .map_err(|_| AmosError::Validation("object byte count overflow".into()))?,
        })
    }
}

impl ObjectStore for LocalFilesystemObjectStore {
    fn stage(&self, key: &str, content: &str, expected_hash: &str) -> Result<PromotedObject> {
        if content_hash(&content)? != expected_hash {
            return Err(AmosError::Validation(
                "staged object content hash does not match the artifact".into(),
            ));
        }
        let path = self.path("staged", key)?;
        if path.exists() {
            return self.inspect(key, path, expected_hash);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        self.inspect(key, path, expected_hash)
    }

    fn promote(&self, key: &str, expected_hash: &str) -> Result<PromotedObject> {
        let target = self.path("objects", key)?;
        if target.exists() {
            return self.inspect(key, target, expected_hash);
        }
        let staged = self.path("staged", key)?;
        self.inspect(key, staged.clone(), expected_hash)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&staged, &target)?;
        self.inspect(key, target, expected_hash)
    }

    fn read(&self, key: &str) -> Result<Option<String>> {
        let path = self.path("objects", key)?;
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read_to_string(path)?))
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn promotion_is_hash_checked_idempotent_and_recovers_after_lost_acknowledgment() {
        let root = TempDir::new().unwrap();
        let store = LocalFilesystemObjectStore::new(root.path()).unwrap();
        let content = "verified artifact";
        let hash = content_hash(&content).unwrap();
        store.stage("tenant/artifact.md", content, &hash).unwrap();
        let promoted = store.promote("tenant/artifact.md", &hash).unwrap();
        assert_eq!(promoted.content_hash, hash);
        assert_eq!(
            store.promote("tenant/artifact.md", &hash).unwrap(),
            promoted
        );
        assert_eq!(
            store.read("tenant/artifact.md").unwrap().as_deref(),
            Some(content)
        );
        assert!(matches!(
            store.stage("../escape", content, &hash),
            Err(AmosError::Validation(_))
        ));
        assert!(matches!(
            store.stage(
                "tenant/tampered.md",
                content,
                &content_hash(&"different").unwrap()
            ),
            Err(AmosError::Validation(_))
        ));
    }
}
