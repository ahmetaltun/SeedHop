use crate::{CoreError, FileOperation, Result};
use std::sync::{Arc, Mutex};

pub(crate) struct Control(Mutex<FileOperation>);

impl Control {
    pub fn snapshot(&self) -> FileOperation {
        self.0.lock().unwrap().clone()
    }

    pub fn cancel(&self, id: &str) -> bool {
        let mut state = self.0.lock().unwrap();
        if state.id != id || !matches!(state.phase.as_str(), "preparing" | "copying") {
            return false;
        }
        state.cancel_requested = true;
        true
    }

    pub fn advance(&self, bytes: u64) -> Result<()> {
        let mut state = self.0.lock().unwrap();
        state.completed_bytes = state.completed_bytes.saturating_add(bytes);
        state.total_bytes = state.total_bytes.max(state.completed_bytes);
        if state.cancel_requested {
            return Err(CoreError::Cancelled);
        }
        Ok(())
    }

    pub fn copying(&self, total: u64) -> Result<()> {
        self.advance(0)?;
        let mut state = self.0.lock().unwrap();
        state.phase = "copying".into();
        state.total_bytes = total;
        Ok(())
    }

    // Serialize cancellation with entering the non-cancellable commit phase.
    // A successful cancellation can never race past this boundary into a save.
    pub fn saving(&self) -> Result<()> {
        let mut state = self.0.lock().unwrap();
        if state.cancel_requested {
            return Err(CoreError::Cancelled);
        }
        state.phase = "saving".into();
        state.total_bytes = state.completed_bytes;
        Ok(())
    }

    pub fn phase(&self, phase: &str) {
        self.0.lock().unwrap().phase = phase.into();
    }
}

pub(crate) struct Guard<'a> {
    slot: &'a Mutex<Option<Arc<Control>>>,
    pub control: Arc<Control>,
}

impl<'a> Guard<'a> {
    // The caller holds the engine mutation lock for the guard's whole lifetime.
    pub fn begin(slot: &'a Mutex<Option<Arc<Control>>>, torrent_id: String, kind: &str) -> Self {
        let control = Arc::new(Control(Mutex::new(FileOperation {
            id: uuid::Uuid::new_v4().to_string(),
            torrent_id,
            kind: kind.into(),
            phase: "preparing".into(),
            completed_bytes: 0,
            total_bytes: 0,
            cancel_requested: false,
        })));
        *slot.lock().unwrap() = Some(control.clone());
        Self { slot, control }
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        let mut slot = self.slot.lock().unwrap();
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &self.control))
        {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_and_commit_have_one_winner_and_old_ids_cannot_cancel_retries() {
        for _ in 0..64 {
            let slot = Mutex::new(None);
            let operation = Guard::begin(&slot, "torrent".into(), "relocate");
            operation.control.copying(100).unwrap();
            let id = operation.control.snapshot().id;
            std::thread::scope(|scope| {
                let commit = scope.spawn(|| operation.control.saving());
                let cancelled = operation.control.cancel(&id);
                assert_eq!(
                    cancelled,
                    matches!(commit.join().unwrap(), Err(CoreError::Cancelled))
                );
            });
            drop(operation);
            assert!(slot.lock().unwrap().is_none());
            let retry = Guard::begin(&slot, "torrent".into(), "relocate");
            assert!(!retry.control.cancel(&id));
            retry.control.copying(100).unwrap();
            retry.control.advance(100).unwrap();
            retry.control.saving().unwrap();
            assert!(!retry.control.cancel(&retry.control.snapshot().id));
        }
    }
}
