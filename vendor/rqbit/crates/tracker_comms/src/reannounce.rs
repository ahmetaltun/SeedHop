//! Ephemeral controls owned by the existing tracker monitor futures.
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use librqbit_core::hash_id::Id20;
use parking_lot::Mutex;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReannounceDisposition {
    Scheduled,
    AlreadyPending,
    Unavailable,
}

#[derive(Clone, Copy)]
pub struct ReannounceSnapshot {
    pub pending: bool,
    pub in_flight: bool,
    pub closed: bool,
    pub manual_allowed_at: u64,
    pub next_announce: u64,
}

struct Schedule {
    regular: Instant,
    manual_allowed: Instant,
    last_attempt: Instant,
    pending: bool,
    in_flight: bool,
    closed: bool,
}

pub struct TrackerControl {
    pub info_hash: Id20,
    pub url: String,
    pub generation: u64,
    schedule: Mutex<Schedule>,
    wake: Notify,
    manual_gate: Arc<Semaphore>,
}

// A monitor can disappear while a caller holds a control. Closing explicitly
// prevents a surviving handle from scheduling work on the retired generation.
pub(crate) struct MonitorLifetime<'a>(
    pub Arc<TrackerControl>,
    pub &'a dyn crate::TorrentStatsProvider,
);
impl Drop for MonitorLifetime<'_> {
    fn drop(&mut self) {
        self.0.close();
        self.1.unregister_control(&self.0);
    }
}

fn epoch_at(deadline: Instant) -> u64 {
    let seconds = deadline.saturating_duration_since(Instant::now()).as_secs();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_add(seconds)
}

impl TrackerControl {
    pub(crate) fn new(info_hash: Id20, url: String, gate: Arc<Semaphore>) -> Self {
        let now = Instant::now();
        Self {
            info_hash,
            url,
            generation: rand::random(),
            schedule: Mutex::new(Schedule {
                regular: now,
                manual_allowed: now,
                last_attempt: now,
                pending: false,
                in_flight: true,
                closed: false,
            }),
            wake: Notify::new(),
            manual_gate: gate,
        }
    }

    pub fn snapshot(&self) -> ReannounceSnapshot {
        let s = self.schedule.lock();
        let next = if s.pending {
            s.regular.min(s.manual_allowed)
        } else {
            s.regular
        };
        ReannounceSnapshot {
            pending: s.pending,
            in_flight: s.in_flight,
            closed: s.closed,
            manual_allowed_at: epoch_at(s.manual_allowed),
            next_announce: epoch_at(next),
        }
    }

    pub fn request(&self) -> ReannounceDisposition {
        let mut s = self.schedule.lock();
        if s.closed {
            return ReannounceDisposition::Unavailable;
        }
        if s.pending || s.in_flight {
            return ReannounceDisposition::AlreadyPending;
        }
        s.pending = true;
        drop(s);
        self.wake.notify_one();
        ReannounceDisposition::Scheduled
    }

    pub fn close(&self) {
        let mut s = self.schedule.lock();
        s.closed = true;
        s.pending = false;
        drop(s);
        self.wake.notify_one();
    }

    /// Claim the current attempt before waiting for a permit, so subsequent
    /// clicks coalesce even when all manual transport slots are occupied.
    pub(crate) async fn begin(&self) -> anyhow::Result<Option<OwnedSemaphorePermit>> {
        let manual = {
            let mut s = self.schedule.lock();
            anyhow::ensure!(!s.closed, "Tracker monitor retired");
            let manual = s.pending;
            s.in_flight = true;
            s.last_attempt = Instant::now();
            manual
        };
        if manual {
            let permit = self.manual_gate.clone().acquire_owned().await?;
            let mut s = self.schedule.lock();
            anyhow::ensure!(!s.closed, "Tracker monitor retired");
            s.pending = false;
            s.last_attempt = Instant::now();
            Ok(Some(permit))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn succeeded(&self, regular: Duration, minimum: Duration) {
        let now = Instant::now();
        let mut s = self.schedule.lock();
        s.regular = now + regular;
        s.manual_allowed = (now + minimum).max(s.last_attempt + Duration::from_secs(60));
        // Remain in flight until peers have entered the existing delivery path.
    }

    pub(crate) fn finish(&self) {
        self.schedule.lock().in_flight = false;
    }

    pub(crate) fn failed(&self, retry: Duration) {
        let mut s = self.schedule.lock();
        s.regular = Instant::now() + retry;
        s.manual_allowed = s
            .manual_allowed
            .max(s.regular)
            .max(s.last_attempt + Duration::from_secs(60));
        s.in_flight = false;
    }

    pub(crate) async fn wait(&self) -> anyhow::Result<()> {
        loop {
            let due = {
                let s = self.schedule.lock();
                anyhow::ensure!(!s.closed, "Tracker monitor retired");
                if s.pending {
                    s.regular.min(s.manual_allowed)
                } else {
                    s.regular
                }
            };
            tokio::select! {
                _ = tokio::time::sleep_until(due) => return Ok(()),
                _ = self.wake.notified() => {}
            }
        }
    }
}

/// Successful lifecycle events are tracked independently for each endpoint.
#[derive(Default)]
pub(crate) struct AnnounceLifecycle {
    started: bool,
    saw_incomplete: bool,
    completed: bool,
    initial_payload: u64,
}
impl AnnounceLifecycle {
    pub(crate) fn new(initial_payload: u64) -> Self {
        Self {
            initial_payload,
            ..Default::default()
        }
    }
    pub(crate) fn event(
        &self,
        complete: bool,
        payload: u64,
    ) -> Option<crate::tracker_comms_http::TrackerRequestEvent> {
        use crate::tracker_comms_http::TrackerRequestEvent::*;
        if !self.started {
            Some(Started)
        } else if (self.saw_incomplete || payload > self.initial_payload)
            && complete
            && !self.completed
        {
            Some(Completed)
        } else {
            None
        }
    }
    pub(crate) fn succeeded(&mut self, complete: bool, payload: u64) {
        if self.started && (self.saw_incomplete || payload > self.initial_payload) && complete {
            self.completed = true;
        }
        self.started = true;
        self.saw_incomplete |= !complete;
    }
}

/// Reject unrepresentable tracker deadlines instead of overflowing the clock.
pub(crate) fn http_intervals(
    interval: u64,
    minimum: Option<u64>,
    forced: Option<Duration>,
) -> anyhow::Result<(Duration, Duration)> {
    let interval = u32::try_from(interval)
        .map_err(|_| anyhow::anyhow!("Tracker returned an unsupported announce interval"))?;
    let minimum = u32::try_from(minimum.unwrap_or(0))
        .map_err(|_| anyhow::anyhow!("Tracker returned an unsupported announce interval"))?;
    let minimum = Duration::from_secs(minimum as u64);
    let regular = forced
        .unwrap_or(Duration::from_secs(interval as u64))
        .max(minimum)
        .max(Duration::from_secs(5));
    Ok((regular, minimum))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control() -> TrackerControl {
        TrackerControl::new(
            Id20::new([1; 20]),
            "https://tracker.test/secret".into(),
            Arc::new(Semaphore::new(4)),
        )
    }

    #[test]
    fn regular_and_minimum_are_distinct_and_invalid_intervals_are_rejected() {
        assert_eq!(
            http_intervals(1800, Some(120), None).unwrap(),
            (Duration::from_secs(1800), Duration::from_secs(120))
        );
        assert_eq!(
            http_intervals(0, Some(0), None).unwrap().0,
            Duration::from_secs(5)
        );
        assert_eq!(
            http_intervals(10, Some(120), Some(Duration::from_secs(1)))
                .unwrap()
                .0,
            Duration::from_secs(120)
        );
        assert!(http_intervals(u64::MAX, None, None).is_err());
        assert!(http_intervals(1, Some(u64::MAX), None).is_err());
    }

    #[tokio::test]
    async fn clicks_coalesce_and_retired_generation_cannot_accept_work() {
        let c = control();
        assert_eq!(c.request(), ReannounceDisposition::AlreadyPending);
        c.succeeded(Duration::from_secs(1800), Duration::from_secs(120));
        c.finish();
        assert_eq!(c.request(), ReannounceDisposition::Scheduled);
        assert_eq!(c.request(), ReannounceDisposition::AlreadyPending);
        let _permit = c.begin().await.unwrap();
        assert_eq!(c.request(), ReannounceDisposition::AlreadyPending);
        c.close();
        assert!(!c.snapshot().pending);
        assert_eq!(c.request(), ReannounceDisposition::Unavailable);
        assert!(c.wait().await.is_err());
    }

    #[test]
    fn pending_deadlines_respect_minimum_cooldown_and_retry() {
        let c = control();
        c.succeeded(Duration::from_secs(1800), Duration::from_secs(120));
        c.finish();
        let before = c.snapshot();
        c.request();
        let pending = c.snapshot();
        assert!(pending.next_announce < before.next_announce);
        assert_eq!(pending.next_announce, pending.manual_allowed_at);
        c.failed(Duration::from_secs(600));
        assert!(c.snapshot().manual_allowed_at >= pending.manual_allowed_at + 479);
        c.succeeded(Duration::from_secs(1800), Duration::from_secs(1800));
        let udp = c.snapshot();
        assert_eq!(udp.next_announce, udp.manual_allowed_at);
    }

    #[tokio::test(start_paused = true)]
    async fn wake_respects_deadlines_and_retry_floor() {
        let c = Arc::new(control());
        c.succeeded(Duration::from_secs(1800), Duration::from_secs(120));
        c.finish();
        let waiter = {
            let c = c.clone();
            tokio::spawn(async move { c.wait().await })
        };
        tokio::task::yield_now().await;
        assert_eq!(c.request(), ReannounceDisposition::Scheduled);
        tokio::time::advance(Duration::from_secs(119)).await;
        assert!(!waiter.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        waiter.await.unwrap().unwrap();
        let _ = c.begin().await.unwrap();
        c.failed(Duration::from_secs(600));
        c.request();
        let waiter = {
            let c = c.clone();
            tokio::spawn(async move { c.wait().await })
        };
        tokio::time::advance(Duration::from_secs(599)).await;
        assert!(!waiter.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        waiter.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn manual_gate_bounds_requests_and_retired_waiters_cannot_dispatch() {
        let gate = Arc::new(Semaphore::new(4));
        let release = Arc::new(Semaphore::new(0));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut controls = Vec::new();
        let mut tasks = Vec::new();
        for _ in 0..6 {
            let c = Arc::new(TrackerControl::new(
                Id20::new([1; 20]),
                "https://tracker.test".into(),
                gate.clone(),
            ));
            c.succeeded(Duration::from_secs(1800), Duration::from_secs(120));
            c.finish();
            c.request();
            controls.push(c.clone());
            let tx = tx.clone();
            let release = release.clone();
            tasks.push(tokio::spawn(async move {
                let permit = c.begin().await;
                if permit.is_ok() {
                    tx.send(()).unwrap();
                    release.acquire().await.unwrap().forget();
                }
                permit.is_ok()
            }));
        }
        for _ in 0..4 {
            rx.recv().await.unwrap();
        }
        assert_eq!(gate.available_permits(), 0);
        assert!(rx.try_recv().is_err());
        for c in &controls {
            assert_eq!(c.request(), ReannounceDisposition::AlreadyPending);
        }
        // Close every generation while some attempts still wait for permits.
        for c in &controls {
            c.close();
        }
        release.add_permits(6);
        let mut started = 0;
        for task in tasks {
            started += usize::from(task.await.unwrap());
        }
        assert_eq!(started, 4);
        assert_eq!(gate.available_permits(), 4);
    }

    #[test]
    fn inherited_metadata_stream_observes_fast_completion_without_replaying_seed_completion() {
        use crate::tracker_comms_http::TrackerRequestEvent::*;
        let mut metadata = AnnounceLifecycle::new(0);
        metadata.succeeded(true, 0); // Unknown metadata was reported as 0/0.
        assert!(matches!(metadata.event(true, 128), Some(Completed)));
        metadata.succeeded(true, 128);
        assert!(metadata.event(true, 128).is_none());
        // Resumed seeds retain lifetime counters; use a per-monitor baseline.
        let mut seed = AnnounceLifecycle::new(128);
        seed.succeeded(true, 128);
        assert!(seed.event(true, 128).is_none());
    }

    #[test]
    fn lifecycle_retries_do_not_commit_events_or_replay_them() {
        use crate::tracker_comms_http::TrackerRequestEvent::*;
        let mut lifecycle = AnnounceLifecycle::default();
        assert!(matches!(lifecycle.event(false, 0), Some(Started)));
        assert!(matches!(lifecycle.event(false, 0), Some(Started)));
        lifecycle.succeeded(false, 0);
        assert!(lifecycle.event(false, 0).is_none());
        assert!(matches!(lifecycle.event(true, 0), Some(Completed)));
        lifecycle.succeeded(true, 0);
        assert!(lifecycle.event(true, 0).is_none());
        let mut seed = AnnounceLifecycle::default();
        seed.succeeded(true, 0);
        assert!(seed.event(true, 0).is_none());
    }
}
