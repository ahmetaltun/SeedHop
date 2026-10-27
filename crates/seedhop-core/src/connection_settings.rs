use crate::{CoreError, Engine, Result, storage};
use librqbit::EncryptionPolicy;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnectionSettings {
    version: u32,
    pub peer_encryption: EncryptionPolicy,
}
impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            version: 1,
            peer_encryption: EncryptionPolicy::Allow,
        }
    }
}
impl ConnectionSettings {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("connection-settings.json");
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(storage::io_error(error)),
        };
        let value: Self = serde_json::from_slice(&bytes)
            .map_err(|error| CoreError::CorruptState(error.to_string()))?;
        if value.version != 1 {
            return Err(CoreError::CorruptState(
                "Unknown connection settings version".into(),
            ));
        }
        Ok(value)
    }
}
pub(crate) fn name(policy: EncryptionPolicy) -> &'static str {
    match policy {
        EncryptionPolicy::Disabled => "disabled",
        EncryptionPolicy::Allow => "allow",
        EncryptionPolicy::Prefer => "prefer",
        EncryptionPolicy::Require => "require",
    }
}
fn parse(value: &str) -> Result<EncryptionPolicy> {
    match value {
        "disabled" => Ok(EncryptionPolicy::Disabled),
        "allow" => Ok(EncryptionPolicy::Allow),
        "prefer" => Ok(EncryptionPolicy::Prefer),
        "require" => Ok(EncryptionPolicy::Require),
        _ => Err(CoreError::InvalidInput(
            "Choose a supported peer encryption policy.".into(),
        )),
    }
}
impl Engine {
    /// Save for the next engine start. Existing connections retain the active policy.
    pub async fn set_peer_encryption(&self, policy: String) -> Result<()> {
        let policy = parse(&policy)?;
        let _operation = self.operations.write().await;
        self.ready()?;
        let settings = ConnectionSettings {
            version: 1,
            peer_encryption: policy,
        };
        let bytes =
            serde_json::to_vec_pretty(&settings).map_err(|e| CoreError::Engine(e.to_string()))?;
        if let Err(error) = storage::atomic_write(
            &self.state_directory.join("connection-settings.json"),
            &bytes,
        ) {
            // A failed directory flush may follow a successful rename. Reflect
            // the readable saved choice, but keep reporting that durability failed.
            if let Ok(saved) = ConnectionSettings::load(&self.state_directory) {
                *self.connection_settings.lock().unwrap() = saved;
            }
            return Err(error);
        }
        *self.connection_settings.lock().unwrap() = settings;
        Ok(())
    }
}
