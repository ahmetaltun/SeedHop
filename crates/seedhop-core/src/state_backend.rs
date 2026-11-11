//! State ownership is explicit: a memory-only engine has no library path or lease.
use crate::{CoreError, Result, storage};
use std::path::{Path, PathBuf};

pub(crate) enum StateBackend {
    Library {
        directory: PathBuf,
        lease: storage::LibraryLease,
    },
    Memory {
        payloads: crate::private_payloads::PrivatePayloads,
    },
}

impl StateBackend {
    pub(crate) fn root(&self) -> Option<&Path> {
        match self {
            Self::Library { directory, .. } => Some(directory),
            Self::Memory { .. } => None,
        }
    }

    pub(crate) fn require_library(&self) -> Result<&Path> {
        self.root().ok_or_else(|| CoreError::Validation {
            code: "session.private_unsupported",
            message: "This operation is unavailable in a private session.".into(),
        })
    }

    pub(crate) fn lease(&self) -> Option<&storage::LibraryLease> {
        match self {
            Self::Library { lease, .. } => Some(lease),
            Self::Memory { .. } => None,
        }
    }

    pub(crate) fn payloads(&self) -> Option<&crate::private_payloads::PrivatePayloads> {
        match self {
            Self::Memory { payloads } => Some(payloads),
            Self::Library { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_backend_cannot_supply_a_library_or_lease() {
        let parent = tempfile::tempdir().unwrap();
        let state = StateBackend::Memory {
            payloads: crate::private_payloads::PrivatePayloads::new(parent.path()).unwrap(),
        };
        assert!(state.root().is_none());
        assert!(state.lease().is_none());
        assert!(matches!(
            state.require_library(),
            Err(CoreError::Validation {
                code: "session.private_unsupported",
                ..
            })
        ));
    }
}
