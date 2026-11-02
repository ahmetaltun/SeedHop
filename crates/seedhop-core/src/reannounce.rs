//! Manual tracker requests resolve opaque IDs against current live monitors.
use crate::{CoreError, Engine, Result, diagnostics};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReannounceSummary {
    pub scheduled: u32,
    pub already_pending: u32,
    pub unavailable: u32,
}

pub(crate) fn target_id(hash: &str, url: &str, generation: u64) -> String {
    diagnostics::tracker_identity(&format!("reannounce\0{generation}\0{hash}\0{url}"))
}

impl Engine {
    pub async fn force_reannounce(
        &self,
        id: String,
        target_ids: Option<Vec<String>>,
        instance_id: String,
    ) -> Result<ReannounceSummary> {
        // Share the existing lifecycle barrier, but never hold it for network I/O.
        let _operation = self.operations.read().await;
        self.ready()?;
        if instance_id != self.instance_id {
            return Err(CoreError::Validation {
                code: "input.reannounce_stale",
                message: "The torrent engine changed. Refresh and try again.".into(),
            });
        }
        if target_ids.as_ref().is_some_and(Vec::is_empty) {
            return Err(CoreError::Validation {
                code: "input.reannounce_empty",
                message: "Select at least one tracker.".into(),
            });
        }
        let torrent = self.handle(&id)?;
        if self.file_operation.lock().unwrap().is_some()
            || self
                .storage_recovery
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|r| r.torrent_id.is_empty() || r.torrent_id == id)
        {
            return Err(CoreError::Validation {
                code: "input.reannounce_unavailable",
                message: "Tracker updates are unavailable for this transfer.".into(),
            });
        }
        let controls = self.session.tracker_controls(&torrent);
        let mut targets = target_ids.map(|ids| ids.into_iter().collect::<HashSet<_>>());
        let mut summary = ReannounceSummary::default();
        let mut seen = HashSet::new();
        for control in controls {
            let key = target_id(
                &control.info_hash.as_string(),
                &control.url,
                control.generation,
            );
            if !seen.insert(key.clone()) || targets.as_ref().is_some_and(|ids| !ids.contains(&key))
            {
                continue;
            }
            if let Some(ids) = targets.as_mut() {
                ids.remove(&key);
            }
            match control.request() {
                librqbit::ReannounceDisposition::Scheduled => summary.scheduled += 1,
                librqbit::ReannounceDisposition::AlreadyPending => summary.already_pending += 1,
                librqbit::ReannounceDisposition::Unavailable => summary.unavailable += 1,
            }
        }
        summary.unavailable += targets.map(|ids| ids.len() as u32).unwrap_or(0);
        if summary.scheduled == 0 && summary.already_pending == 0 && summary.unavailable == 0 {
            return Err(CoreError::Validation {
                code: "input.reannounce_unavailable",
                message: "Tracker updates are unavailable for this transfer.".into(),
            });
        }
        Ok(summary)
    }
}
