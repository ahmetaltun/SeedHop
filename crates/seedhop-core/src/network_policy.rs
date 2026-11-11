//! Persistent route selection, checked before the session can restore any network work.
use crate::{CoreError, Engine, Result, storage};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::Path,
    sync::atomic::Ordering,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkInterface {
    pub name: String,
    pub index: u32,
    pub addresses: Vec<IpAddr>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkPolicy {
    pub version: u32,
    pub interface: Option<NetworkInterface>,
    pub dns_server: Option<IpAddr>,
    #[serde(default)]
    pub allow_local_network: bool,
}
impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            interface: None,
            dns_server: None,
            allow_local_network: false,
        }
    }
}

pub fn network_interfaces() -> Result<Vec<NetworkInterface>> {
    let mut head = std::ptr::null_mut();
    // SAFETY: getifaddrs initializes a list; the guard frees it after traversal.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(storage::io_error(std::io::Error::last_os_error()));
    }
    struct List(*mut libc::ifaddrs);
    impl Drop for List {
        fn drop(&mut self) {
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
    let _list = List(head);
    let mut interfaces: BTreeMap<String, NetworkInterface> = BTreeMap::new();
    let mut cursor = head;
    while !cursor.is_null() {
        // SAFETY: cursor and its non-null address belong to the live getifaddrs list.
        let item = unsafe { &*cursor };
        cursor = item.ifa_next;
        if item.ifa_addr.is_null() || item.ifa_flags & libc::IFF_UP as u32 == 0 {
            continue;
        }
        let name = unsafe { CStr::from_ptr(item.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let family = unsafe { (*item.ifa_addr).sa_family } as i32;
        let ip = match family {
            libc::AF_INET => {
                let address = unsafe { &*(item.ifa_addr as *const libc::sockaddr_in) };
                IpAddr::V4(Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()))
            }
            libc::AF_INET6 => {
                let address = unsafe { &*(item.ifa_addr as *const libc::sockaddr_in6) };
                IpAddr::V6(Ipv6Addr::from(address.sin6_addr.s6_addr))
            }
            _ => continue,
        };
        let index = unsafe { libc::if_nametoindex(item.ifa_name) };
        if index == 0 {
            continue;
        }
        interfaces
            .entry(name.clone())
            .or_insert_with(|| NetworkInterface {
                name,
                index,
                addresses: vec![],
            })
            .addresses
            .push(ip);
    }
    for interface in interfaces.values_mut() {
        interface.addresses.sort();
        interface.addresses.dedup();
    }
    Ok(interfaces.into_values().collect())
}

impl NetworkPolicy {
    pub fn load(root: &Path) -> Result<Self> {
        let bytes = match std::fs::read(root.join("network-policy.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(storage::io_error(error)),
        };
        let policy: Self = serde_json::from_slice(&bytes)
            .map_err(|_| CoreError::CorruptState("Invalid network protection settings".into()))?;
        policy.validate()?;
        Ok(policy)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.interface.is_some() != self.dns_server.is_some() {
            return Err(CoreError::InvalidInput(
                "Invalid network protection settings.".into(),
            ));
        }
        if let Some(interface) = &self.interface
            && (interface.index == 0
                || interface.addresses.is_empty()
                || interface.name.len() > 32
                || CString::new(interface.name.as_str()).is_err())
        {
            return Err(CoreError::InvalidInput(
                "Select an available network interface.".into(),
            ));
        }
        if let Some(server) = self.dns_server
            && (server.is_unspecified() || server.is_multicast() || server.is_loopback())
        {
            return Err(CoreError::InvalidInput(
                "Choose a numeric DNS server reachable through the selected connection.".into(),
            ));
        }
        Ok(())
    }
    pub fn check_available(&self) -> Result<()> {
        self.validate()?;
        if let Some(selected) = &self.interface
            && !network_interfaces()?.contains(selected)
        {
            return Err(CoreError::NetworkBlocked);
        }
        Ok(())
    }
    fn save(&self, root: &Path) -> Result<()> {
        self.validate()?;
        crate::connection_settings::ConnectionSettings::require_network_policy(root)?;
        storage::atomic_write(
            &root.join("network-policy.json"),
            &serde_json::to_vec_pretty(self).map_err(crate::adapter::engine_error)?,
        )?;
        crate::connection_settings::ConnectionSettings::finish_network_policy(root)
    }
}

pub fn network_preferences(root: &Path) -> Result<String> {
    let saved = saved_policy(root);
    let interfaces = network_interfaces()?;
    serde_json::to_string(&serde_json::json!({ "saved": saved, "interfaces": interfaces, "active": null, "blocked": true }))
        .map_err(crate::adapter::engine_error)
}

fn saved_policy(root: &Path) -> Option<NetworkPolicy> {
    let settings = crate::connection_settings::ConnectionSettings::load(root).ok()?;
    if settings.network_policy_pending
        || (settings.requires_network_policy() && !root.join("network-policy.json").is_file())
    {
        return None;
    }
    NetworkPolicy::load(root).ok()
}

/// Only used without an engine. The same library lease prevents concurrent writers.
pub fn save_network_preferences(root: &Path, json: &str) -> Result<()> {
    let _lease = storage::LibraryLease::acquire(root)?;
    let policy: NetworkPolicy = serde_json::from_str(json)
        .map_err(|_| CoreError::InvalidInput("Invalid network protection settings.".into()))?;
    policy.check_available()?;
    policy.save(root)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateProtection {
    pub network_policy: NetworkPolicy,
    pub peer_encryption: librqbit::EncryptionPolicy,
}

pub fn offline_private_protection(root: &Path) -> Result<PrivateProtection> {
    let _lease = storage::LibraryLease::acquire_existing(root)?;
    private_protection(root)
}

fn private_protection(root: &Path) -> Result<PrivateProtection> {
    let settings = crate::connection_settings::ConnectionSettings::load(root)?;
    let network_policy = saved_policy(root).ok_or(CoreError::NetworkBlocked)?;
    network_policy.check_available()?;
    Ok(PrivateProtection {
        network_policy,
        peer_encryption: settings.peer_encryption,
    })
}

impl Engine {
    pub async fn private_protection(&self) -> Result<PrivateProtection> {
        let _operation = self.operations.read().await;
        self.ready()?;
        let protection = private_protection(self.state.require_library()?)?;
        if protection.network_policy != self.network_policy
            || protection.peer_encryption != self.active_peer_encryption
        {
            return Err(CoreError::NetworkBlocked);
        }
        Ok(protection)
    }

    pub fn network_info(&self) -> Result<String> {
        let saved = self.state.root().and_then(saved_policy);
        serde_json::to_string(&serde_json::json!({ "saved": saved, "active": self.network_policy,
            "interfaces": network_interfaces()?, "blocked": self.network_blocked.load(Ordering::SeqCst) }))
            .map_err(crate::adapter::engine_error)
    }
    pub async fn set_network_policy(&self, json: String) -> Result<()> {
        let root = self.state.require_library()?;
        let _operation = self.operations.write().await;
        if self.stopping.load(Ordering::SeqCst) {
            return Err(CoreError::Stopping);
        }
        let policy: NetworkPolicy = serde_json::from_str(&json)
            .map_err(|_| CoreError::InvalidInput("Invalid network protection settings.".into()))?;
        policy.check_available()?;
        // Stop first, including when the subsequent save fails. Never leave direct traffic
        // running after the user has requested protection or a different protected route.
        self.network_blocked.store(true, Ordering::SeqCst);
        self.session
            .stop_and_wait()
            .await
            .map_err(crate::adapter::engine_error)?;
        policy.save(root)
    }
}
