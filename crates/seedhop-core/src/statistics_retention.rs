//! Statistics retention is library policy, separate from payload resume state.
use crate::{CoreError, Engine, Result, connection_settings::ConnectionSettings, storage};
use serde::Serialize;
use std::{path::Path, sync::atomic::Ordering};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    remember_statistics: bool,
    clear_pending: bool,
}

pub fn statistics_preferences(root: &Path) -> Result<String> {
    let saved = ConnectionSettings::load(root)?;
    serde_json::to_string(&Preferences {
        remember_statistics: saved.remembers_statistics(),
        clear_pending: saved.statistics_clear_pending,
    })
    .map_err(crate::adapter::engine_error)
}

fn save(root: &Path, remember: bool) -> Result<()> {
    let mut saved = ConnectionSettings::load(root)?;
    // Never enable persistence while an earlier requested purge is unfinished.
    saved.complete_statistics_clear(root)?;
    if remember && saved.remembers_statistics() {
        return Ok(());
    }
    saved.set_statistics_retention(remember);
    // Version 4 and the pending marker precede deletion. Older readers refuse
    // the library rather than silently resuming historical accounting.
    saved.save(root)?;
    saved.complete_statistics_clear(root)
}

pub fn save_statistics_preferences(root: &Path, remember: bool) -> Result<String> {
    let _lease = storage::LibraryLease::acquire(root)?;
    save(root, remember)?;
    statistics_preferences(root)
}

impl Engine {
    pub fn statistics_preferences(&self) -> Result<String> {
        statistics_preferences(&self.state_directory)
    }

    pub async fn set_statistics_retention(&self, remember: bool) -> Result<String> {
        let _operation = self.operations.write().await;
        if self.stopping.load(Ordering::SeqCst) {
            return Err(CoreError::Stopping);
        }
        let saved = save(&self.state_directory, remember);
        // Sample while the old accounting mode is still active, then establish
        // the boundary for future retained totals without resetting session data.
        self.sample_accounting();
        let live_ids = self.session.with_torrents(|torrents| {
            torrents
                .map(|(_, torrent)| crate::adapter::torrent_id(torrent))
                .collect::<std::collections::HashSet<_>>()
        });
        let readable = ConnectionSettings::load(&self.state_directory);
        let mut accounting = self.accounting.lock().unwrap();
        accounting.entries.retain(|id, _| live_ids.contains(id));
        match readable {
            Ok(settings) => {
                accounting.set_retention(settings.remembers_statistics());
                *self.connection_settings.lock().unwrap() = settings;
            }
            Err(_) => accounting.set_retention(false),
        }
        let result = saved.and_then(|_| accounting.checkpoint(&self.state_directory, true));
        *self.statistics_warning.lock().unwrap() = result.as_ref().err().map(|_| {
            "Statistics retention could not be fully saved or cleared. Review the saved choice and retry.".into()
        });
        result?;
        statistics_preferences(&self.state_directory)
    }
}
