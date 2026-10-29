use crate::{CoreError, Engine, Result, adapter, storage};
use serde::{Deserialize, Serialize};
use std::{num::NonZeroU32, path::Path};

/// Monday is bit 0. End is exclusive; equal times mean a full day.
/// Overnight windows belong to their starting day, including across Sunday.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleRule {
    pub days: u8,
    pub start_minute: u16,
    pub end_minute: u16,
    pub download_limit: u32,
    pub upload_limit: u32,
    pub suspend: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleConfig {
    pub enabled: bool,
    pub rules: Vec<ScheduleRule>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedSchedule {
    version: u32,
    config: ScheduleConfig,
}

pub(crate) struct Scheduling {
    pub config: ScheduleConfig,
    pub normal_limits: [u32; 2],
    pub active: Option<usize>,
}

impl ScheduleRule {
    fn contains(&self, day: usize, minute: u16) -> bool {
        let selected = |day: usize| self.days & (1u8 << day) != 0;
        if self.start_minute < self.end_minute {
            selected(day) && minute >= self.start_minute && minute < self.end_minute
        } else {
            (selected(day) && minute >= self.start_minute)
                || (selected((day + 6) % 7) && minute < self.end_minute)
        }
    }
}

impl ScheduleConfig {
    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.rules.is_empty() {
            return Err(CoreError::InvalidInput(
                "Add a window before enabling the schedule.".into(),
            ));
        }
        if self.rules.len() > 32 {
            return Err(CoreError::InvalidInput(
                "Use at most 32 schedule windows.".into(),
            ));
        }
        let mut occupied = vec![false; 7 * 1440];
        for rule in &self.rules {
            if rule.days == 0
                || rule.days > 127
                || rule.start_minute >= 1440
                || rule.end_minute >= 1440
            {
                return Err(CoreError::InvalidInput(
                    "Choose at least one day and valid schedule times.".into(),
                ));
            }
            for day in 0..7 {
                for minute in 0..1440 {
                    if rule.contains(day, minute) {
                        let slot = &mut occupied[day * 1440 + minute as usize];
                        if *slot {
                            return Err(CoreError::InvalidInput(
                                "Schedule windows overlap. Adjust their days or times.".into(),
                            ));
                        }
                        *slot = true;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn load(root: &Path) -> Result<Self> {
        let bytes = match std::fs::read(root.join("schedule.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(storage::io_error(error)),
        };
        let saved: SavedSchedule =
            serde_json::from_slice(&bytes).map_err(|e| CoreError::CorruptState(e.to_string()))?;
        if saved.version != 1 {
            return Err(CoreError::CorruptState("Unknown schedule version".into()));
        }
        saved
            .config
            .validate()
            .map_err(|e| CoreError::CorruptState(e.to_string()))?;
        Ok(saved.config)
    }

    fn active_at(&self, day: usize, minute: u16) -> Option<usize> {
        self.enabled
            .then(|| {
                self.rules
                    .iter()
                    .position(|rule| rule.contains(day, minute))
            })
            .flatten()
    }
}

fn local_week_minute() -> (usize, u16) {
    // Read the current local calendar each tick. Sleep, wall-clock changes and DST
    // select the current window rather than replaying missed transitions.
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    let result = unsafe { libc::localtime_r(&now, local.as_mut_ptr()) };
    if result.is_null() {
        return (0, 0);
    }
    let local = unsafe { local.assume_init() };
    (
        ((local.tm_wday + 6) % 7) as usize,
        (local.tm_hour * 60 + local.tm_min) as u16,
    )
}

impl Engine {
    pub async fn set_schedule(&self, config: ScheduleConfig) -> Result<()> {
        config.validate()?;
        let _operation = self.operations.write().await;
        self.ready()?;
        let bytes = serde_json::to_vec_pretty(&SavedSchedule {
            version: 1,
            config: config.clone(),
        })
        .map_err(adapter::engine_error)?;
        if let Err(error) =
            storage::atomic_write(&self.state_directory.join("schedule.json"), &bytes)
        {
            // A directory flush can fail after rename. Reflect readable disk state
            // while reporting the failed durability guarantee to the caller.
            if let Ok(saved) = ScheduleConfig::load(&self.state_directory) {
                self.replace_schedule_config(saved);
                self.reconcile_schedule();
            }
            return Err(error);
        }
        self.replace_schedule_config(config);
        self.apply_scheduling().await
    }

    fn replace_schedule_config(&self, config: ScheduleConfig) {
        let (day, minute) = local_week_minute();
        let mut state = self.scheduling.lock().unwrap();
        // Snapshot readers do not take the operations lock. Replace the derived
        // index together with its rules so no reader can index an old window.
        state.active = config.active_at(day, minute);
        state.config = config;
    }

    pub(crate) async fn apply_scheduling(&self) -> Result<()> {
        self.reconcile_schedule();
        let result = self.reconcile_queue().await;
        *self.scheduling_warning.lock().unwrap() = result.as_ref().err().map(|error| {
            format!("Transfer scheduling could not be fully applied: {error} Check library storage access. SeedHop will retry automatically.")
        });
        result
    }

    pub(crate) fn schedule_suspended(&self) -> bool {
        let state = self.scheduling.lock().unwrap();
        state
            .active
            .is_some_and(|index| state.config.rules[index].suspend)
    }

    pub(crate) fn reconcile_schedule(&self) {
        let (day, minute) = local_week_minute();
        self.apply_schedule_at(day, minute);
    }

    fn apply_schedule_at(&self, day: usize, minute: u16) {
        let mut state = self.scheduling.lock().unwrap();
        state.active = state.config.active_at(day, minute);
        let limits = state
            .active
            .map(|index| {
                let rule = &state.config.rules[index];
                [rule.download_limit, rule.upload_limit]
            })
            .unwrap_or(state.normal_limits);
        if self.session.ratelimits.get_download_bps() != NonZeroU32::new(limits[0]) {
            self.session
                .ratelimits
                .set_download_bps(NonZeroU32::new(limits[0]));
        }
        if self.session.ratelimits.get_upload_bps() != NonZeroU32::new(limits[1]) {
            self.session
                .ratelimits
                .set_upload_bps(NonZeroU32::new(limits[1]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(days: u8, start: u16, end: u16) -> ScheduleRule {
        ScheduleRule {
            days,
            start_minute: start,
            end_minute: end,
            download_limit: 1200,
            upload_limit: 500,
            suspend: false,
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transitions_apply_current_window_without_replaying_missed_windows() {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let engine = Engine::open(crate::EngineConfig {
            state_directory: state.path().display().to_string(),
            download_directory: files.path().display().to_string(),
            allow_network_discovery: false,
            listen_port: 0,
        })
        .await
        .unwrap();
        engine.set_limits(9000, 8000).await.unwrap();
        {
            // Hold the same lock used by the production timer: deterministic
            // calendar transitions cannot race the host's real wall clock.
            let _operation = engine.operations.write().await;
            engine.scheduling.lock().unwrap().config = ScheduleConfig {
                enabled: true,
                rules: vec![
                    rule(1, 60, 120),
                    ScheduleRule {
                        suspend: true,
                        ..rule(1, 120, 180)
                    },
                ],
            };
            for (day, minute, active, suspended, down) in [
                (0, 59, false, false, 9000),
                (0, 60, true, false, 1200),
                (0, 120, true, true, 1200),
                (0, 180, false, false, 9000),
                (0, 90, true, false, 1200),
                (3, 600, false, false, 9000),
            ] {
                engine.apply_schedule_at(day, minute);
                let snapshot = engine.snapshot().unwrap();
                assert_eq!(snapshot.schedule_active, active);
                assert_eq!(engine.schedule_suspended(), suspended);
                assert_eq!(snapshot.download_limit, down);
                assert_eq!(snapshot.normal_download_limit, 9000);
            }
        }
        engine.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scheduled_queue_changes_do_not_wait_for_read_only_inspection() {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let engine = Engine::open(crate::EngineConfig {
            state_directory: state.path().display().to_string(),
            download_directory: files.path().display().to_string(),
            allow_network_discovery: false,
            listen_port: 0,
        })
        .await
        .unwrap();
        engine
            .add(
                crate::adapter::create_fixture("pending", b"content"),
                files.path().display().to_string(),
                vec![0],
                false,
                true,
            )
            .await
            .unwrap();
        engine
            .set_schedule(ScheduleConfig {
                enabled: true,
                rules: vec![rule(127, 0, 0)],
            })
            .await
            .unwrap();
        {
            // inspect() retains this shared lock while waiting for metadata.
            // A calendar transition must still reach the existing transfer.
            let _inspection = engine.operations.read().await;
            engine.scheduling.lock().unwrap().config.rules[0].suspend = true;
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    if engine.snapshot().unwrap().torrents[0].state == "scheduled" {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("read-only metadata inspection blocked the scheduler");
        }
        engine.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn replacing_active_rules_is_safe_for_concurrent_snapshot_readers() {
        let state = tempfile::tempdir().unwrap();
        let files = tempfile::tempdir().unwrap();
        let engine = Engine::open(crate::EngineConfig {
            state_directory: state.path().display().to_string(),
            download_directory: files.path().display().to_string(),
            allow_network_discovery: false,
            listen_port: 0,
        })
        .await
        .unwrap();
        let (day, _) = local_week_minute();
        let two = ScheduleConfig {
            enabled: true,
            rules: vec![rule(127 ^ (1 << day), 0, 0), rule(1 << day, 0, 0)],
        };
        let one = ScheduleConfig {
            enabled: true,
            rules: vec![rule(127, 0, 0)],
        };
        engine.set_schedule(two.clone()).await.unwrap();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let snapshots = tokio::spawn({
            let engine = engine.clone();
            let done = done.clone();
            async move {
                let mut count = 0;
                while !done.load(std::sync::atomic::Ordering::SeqCst) {
                    let snapshot = engine.snapshot().unwrap();
                    assert!(snapshot.schedule.enabled && snapshot.schedule_active);
                    count += 1;
                    tokio::task::yield_now().await;
                }
                count
            }
        });
        for _ in 0..50 {
            engine.set_schedule(one.clone()).await.unwrap();
            engine.set_schedule(two.clone()).await.unwrap();
        }
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(snapshots.await.unwrap() > 0);
        engine.shutdown().await.unwrap();
    }

    #[test]
    fn boundaries_overnight_week_wrap_and_full_day() {
        let config = ScheduleConfig {
            enabled: true,
            rules: vec![rule(64, 1380, 60)],
        };
        config.validate().unwrap();
        assert_eq!(config.active_at(6, 1379), None);
        assert_eq!(config.active_at(6, 1380), Some(0));
        assert_eq!(config.active_at(0, 59), Some(0));
        assert_eq!(config.active_at(0, 60), None);
        assert_eq!(config.active_at(1, 30), None);
        let full = rule(1, 600, 600);
        assert!(full.contains(0, 600));
        assert!(full.contains(1, 599));
        assert!(!full.contains(1, 600));
        let disabled = ScheduleConfig {
            enabled: false,
            ..config
        };
        assert_eq!(disabled.active_at(0, 30), None);
    }
    #[test]
    fn reject_overlaps_including_previous_day_and_allow_adjacent_windows() {
        for rules in [
            vec![rule(64, 1380, 60), rule(1, 0, 30)],
            vec![rule(1, 0, 0), rule(1, 600, 700)],
            vec![rule(0, 0, 60)],
            vec![rule(128, 0, 60)],
            vec![rule(1, 0, 1440)],
        ] {
            assert!(
                ScheduleConfig {
                    enabled: false,
                    rules
                }
                .validate()
                .is_err()
            );
        }
        ScheduleConfig {
            enabled: true,
            rules: vec![rule(127, 0, 60), rule(127, 60, 0)],
        }
        .validate()
        .unwrap();
    }
}
