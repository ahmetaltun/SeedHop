use crate::{CoreError, Engine, Result, storage};
use librqbit::EncryptionPolicy;
use serde::{Deserialize, Serialize};
use std::path::Path;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnectionSettings {
    version: u32,
    pub peer_encryption: EncryptionPolicy,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) network_policy_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remember_statistics: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    network_policy_required: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) statistics_clear_pending: bool,
}
impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            version: 1,
            peer_encryption: EncryptionPolicy::Allow,
            network_policy_pending: false,
            remember_statistics: None,
            network_policy_required: None,
            statistics_clear_pending: false,
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
        // Version 3 deliberately disables a library during sandbox migration or
        // rollback. Never admit it as an active settings format: the other copy
        // may own the same payload files.
        if !matches!(value.version, 1 | 2 | 4) {
            return Err(CoreError::CorruptState(
                "Unknown connection settings version".into(),
            ));
        }
        if (value.version == 4
            && (value.remember_statistics.is_none() || value.network_policy_required.is_none()))
            || (value.version != 4
                && (value.remember_statistics.is_some()
                    || value.network_policy_required.is_some()
                    || value.statistics_clear_pending))
            || (value.statistics_clear_pending && value.remembers_statistics())
        {
            return Err(CoreError::CorruptState(
                "Incomplete statistics retention settings".into(),
            ));
        }
        Ok(value)
    }
    pub(crate) fn requires_network_policy(&self) -> bool {
        self.version == 2 || self.network_policy_required.unwrap_or(false)
    }
    pub(crate) fn remembers_statistics(&self) -> bool {
        self.remember_statistics.unwrap_or(true)
    }
    pub(crate) fn set_statistics_retention(&mut self, remember: bool) {
        let network_required = self.requires_network_policy();
        self.version = 4;
        self.network_policy_required = Some(network_required);
        self.remember_statistics = Some(remember);
        self.statistics_clear_pending = !remember;
    }
    pub(crate) fn save(&self, root: &Path) -> Result<()> {
        storage::atomic_write(
            &root.join("connection-settings.json"),
            &serde_json::to_vec_pretty(self).map_err(crate::adapter::engine_error)?,
        )
    }
    pub(crate) fn complete_statistics_clear(&mut self, root: &Path) -> Result<()> {
        if self.statistics_clear_pending {
            storage::atomic_write(&root.join("statistics.json"), b"{}")?;
            self.statistics_clear_pending = false;
            self.save(root)?;
        }
        Ok(())
    }

    /// Old clients reject version 2 before opening a network session. Write this
    /// barrier before the route policy so a partial save cannot weaken protection.
    pub(crate) fn require_network_policy(root: &Path) -> Result<()> {
        let mut settings = Self::load(root)?;
        if settings.version == 4 {
            settings.network_policy_required = Some(true);
        } else {
            settings.version = 2;
        }
        settings.network_policy_pending = true;
        storage::atomic_write(
            &root.join("connection-settings.json"),
            &serde_json::to_vec_pretty(&settings).map_err(crate::adapter::engine_error)?,
        )
    }
    pub(crate) fn finish_network_policy(root: &Path) -> Result<()> {
        let mut settings = Self::load(root)?;
        settings.network_policy_pending = false;
        storage::atomic_write(
            &root.join("connection-settings.json"),
            &serde_json::to_vec_pretty(&settings).map_err(crate::adapter::engine_error)?,
        )
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
        _ => Err(CoreError::Validation {
            code: "input.choose_a_supported_peer_encryption_policy",
            message: "Choose a supported peer encryption policy.".into(),
        }),
    }
}
impl Engine {
    /// Save for the next engine start. Existing connections retain the active policy.
    pub async fn set_peer_encryption(&self, policy: String) -> Result<()> {
        let policy = parse(&policy)?;
        let _operation = self.operations.write().await;
        self.ready()?;
        let mut settings = ConnectionSettings::load(&self.state_directory)?;
        settings.peer_encryption = policy;
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
