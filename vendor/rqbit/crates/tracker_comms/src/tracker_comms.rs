use std::collections::HashSet;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use std::net::SocketAddrV6;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use anyhow::bail;
use backon::ExponentialBuilder;
use backon::Retryable;
use futures::FutureExt;
use futures::StreamExt;
use futures::future::Either;
use futures::stream::BoxStream;
use futures::stream::FuturesUnordered;
use tracing::Instrument;
use tracing::debug;
use tracing::debug_span;
use tracing::trace;
use tracing::trace_span;
use url::Url;

use crate::TrackerControl;
use crate::reannounce::{AnnounceLifecycle, MonitorLifetime, http_intervals};
use crate::tracker_comms_http;
use crate::tracker_comms_udp;
use crate::tracker_comms_udp::UdpTrackerClient;
use librqbit_core::hash_id::Id20;

pub struct TrackerComms {
    info_hash: Id20,
    peer_id: Id20,
    stats: Box<dyn TorrentStatsProvider>,
    force_tracker_interval: Option<Duration>,
    tx: Sender,
    // This MUST be set as trackers don't work with 0 port.
    announce_port: u16,
    reqwest_client: reqwest::Client,
    key: u32,
    block_local_network: bool,
}

#[derive(Default)]
pub enum TrackerCommsStatsState {
    #[default]
    None,
    Initializing,
    Paused,
    Live,
}

#[derive(Default)]
pub struct TrackerCommsStats {
    pub uploaded_bytes: u64,
    /// Payload received since torrent creation, independent of verified progress.
    /// A monitor baseline detects completion after metadata-only registration.
    pub payload_downloaded_bytes: u64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub torrent_state: TrackerCommsStatsState,
}

impl TrackerCommsStats {
    pub fn get_left_to_download_bytes(&self) -> u64 {
        let total = self.total_bytes;
        let down = self.downloaded_bytes;
        if total >= down {
            return total - down;
        }
        0
    }

    pub fn is_completed(&self) -> bool {
        self.downloaded_bytes >= self.total_bytes
    }
}

/// An observed announce attempt. URL and endpoint are internal identities;
/// consumers must redact credentials before display or diagnostic export.
#[derive(Clone, Debug, serde::Serialize)]
pub struct TrackerObservation {
    pub url: String,
    pub endpoint: Option<String>,
    pub state: String,
    pub message: Option<String>,
    pub last_announce: Option<u64>,
    pub next_announce: Option<u64>,
    pub peers: Option<u64>,
    pub seeds: Option<u64>,
    pub leechers: Option<u64>,
}

impl TrackerObservation {
    fn new(url: &Url, endpoint: Option<SocketAddr>, state: &str) -> Self {
        Self {
            url: url.to_string(),
            endpoint: endpoint.map(|addr| addr.to_string()),
            state: state.into(),
            message: None,
            last_announce: Some(epoch_seconds()),
            next_announce: None,
            peers: None,
            seeds: None,
            leechers: None,
        }
    }

    fn retry(url: &Url, endpoint: Option<SocketAddr>, delay: Duration, message: String) -> Self {
        let mut observation = Self::new(url, endpoint, "error");
        observation.message = Some(message);
        observation.next_announce = Some(epoch_seconds().saturating_add(delay.as_secs()));
        observation
    }
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// reqwest errors can contain the complete credential-bearing announce URL.
// Only deliberately constructed tracker replies are safe to retain here;
// applications still need to sanitize tracker-supplied failure text.
fn http_failure_message(error: &anyhow::Error) -> String {
    let message = error.to_string();
    if message.starts_with("Tracker rejected announce: ")
        || message.starts_with("Tracker returned HTTP ")
    {
        message
    } else {
        "Tracker request failed; another attempt is scheduled.".into()
    }
}

pub trait TorrentStatsProvider: Send + Sync {
    fn get(&self) -> TrackerCommsStats;

    /// Called synchronously before peer delivery. Implementations must remain
    /// lightweight and must not perform disk or network I/O.
    fn observe(&self, _observation: TrackerObservation) {}

    fn register_control(&self, _control: std::sync::Weak<TrackerControl>) {}

    fn unregister_control(&self, _control: &Arc<TrackerControl>) {}

    fn manual_gate(&self) -> Arc<tokio::sync::Semaphore> {
        Arc::new(tokio::sync::Semaphore::new(4))
    }
}

impl TorrentStatsProvider for () {
    fn get(&self) -> TrackerCommsStats {
        Default::default()
    }
}

type Sender = tokio::sync::mpsc::Sender<SocketAddr>;

enum SupportedTracker {
    Udp(Url),
    Http(Url),
}

impl std::fmt::Debug for SupportedTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportedTracker::Udp(u) => std::fmt::Display::fmt(u, f),
            SupportedTracker::Http(u) => std::fmt::Display::fmt(u, f),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum UdpTrackerResolveResult {
    One(SocketAddr),
    Two(SocketAddrV4, SocketAddrV6),
}

async fn udp_tracker_to_socket_addrs(
    host: url::Host<&str>,
    port: u16,
    dns: Option<&crate::BoundDns>,
) -> anyhow::Result<UdpTrackerResolveResult> {
    let res = match host {
        url::Host::Domain(name) => {
            // Use the first IPv4 and the first IPv6 addresses only.

            let mut v4: Option<SocketAddrV4> = None;
            let mut v6: Option<SocketAddrV6> = None;
            let addresses = match dns {
                Some(dns) => dns.lookup(name, port).await?,
                None => tokio::net::lookup_host((name, port)).await?.collect(),
            };
            for addr in addresses {
                match (v4, v6, addr) {
                    (None, _, SocketAddr::V4(addr)) => v4 = Some(addr),
                    (_, None, SocketAddr::V6(addr)) => v6 = Some(addr),
                    _ => continue,
                }
            }
            let res = match (v4, v6) {
                (Some(v4), Some(v6)) => UdpTrackerResolveResult::Two(v4, v6),
                (Some(v4), None) => UdpTrackerResolveResult::One(v4.into()),
                (None, Some(v6)) => UdpTrackerResolveResult::One(v6.into()),
                _ => anyhow::bail!("zero addresses returned looking up {name}"),
            };
            trace!(?res, "resolved");
            res
        }
        url::Host::Ipv4(addr) => UdpTrackerResolveResult::One((addr, port).into()),
        url::Host::Ipv6(addr) => UdpTrackerResolveResult::One((addr, port).into()),
    };
    Ok(res)
}

impl TrackerComms {
    // TODO: fix too many args
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        info_hash: Id20,
        peer_id: Id20,
        trackers: HashSet<Url>,
        stats: Box<dyn TorrentStatsProvider>,
        force_interval: Option<Duration>,
        announce_port: u16,
        reqwest_client: reqwest::Client,
        udp_client: UdpTrackerClient,
    ) -> Option<BoxStream<'static, SocketAddr>> {
        let trackers = trackers
            .into_iter()
            .filter_map(|t| match t.scheme() {
                "http" | "https" => Some(SupportedTracker::Http(t)),
                "udp" => Some(SupportedTracker::Udp(t)),
                _ => {
                    debug!("unsupported tracker URL: {}", t);
                    None
                }
            })
            .collect::<Vec<_>>();
        if trackers.is_empty() {
            debug!(?info_hash, "trackers list is empty");
            return None;
        }

        tracing::trace!(?trackers);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<SocketAddr>(16);

        let s = async_stream::stream! {
            use futures::StreamExt;
            let comms = Arc::new(Self {
                info_hash,
                peer_id,
                stats,
                force_tracker_interval: force_interval,
                tx,
                announce_port,
                reqwest_client,
                key: rand::random(),
                block_local_network: udp_client.block_local_network,
            });
            let mut futures = FuturesUnordered::new();
            for tracker in trackers {
                let url = match &tracker { SupportedTracker::Http(url) | SupportedTracker::Udp(url) => url.to_string() };
                let control = Arc::new(TrackerControl::new(info_hash, url, comms.stats.manual_gate()));
                comms.stats.register_control(Arc::downgrade(&control));
                futures.push(comms.add_tracker(tracker, &udp_client, control))
            }
            while !(futures.is_empty()) {
                tokio::select! {
                    addr = rx.recv() => {
                        if let Some(addr) = addr {
                            yield addr;
                        }
                    }
                    e = futures.next(), if !futures.is_empty() => {
                        if let Some(Err(e)) = e {
                            debug!("error: {e}");
                        }
                    }
                }
            }
        };

        Some(s.boxed())
    }

    fn add_tracker(
        &self,
        url: SupportedTracker,
        client: &UdpTrackerClient,
        control: Arc<TrackerControl>,
    ) -> Either<
        impl std::future::Future<Output = anyhow::Result<()>> + '_ + Send,
        impl std::future::Future<Output = anyhow::Result<()>> + '_ + Send,
    > {
        let info_hash = self.info_hash;
        match url {
            SupportedTracker::Udp(url) => {
                let span = debug_span!(parent: None, "udp_tracker", tracker = %url, info_hash = ?info_hash);
                self.task_single_tracker_monitor_udp(url, client.clone(), control)
                    .instrument(span)
                    .right_future()
            }
            SupportedTracker::Http(url) => {
                let span = debug_span!(
                    parent: None,
                    "http_tracker",
                    tracker = %url,
                    info_hash = ?info_hash
                );
                self.task_single_tracker_monitor_http(url, control)
                    .instrument(span)
                    .left_future()
            }
        }
    }

    async fn task_single_tracker_monitor_http(
        &self,
        tracker_url: Url,
        control: Arc<TrackerControl>,
    ) -> anyhow::Result<()> {
        let _lifetime = MonitorLifetime(control.clone(), self.stats.as_ref());
        trace!(url=%tracker_url, "starting monitor");
        let mut lifecycle = AnnounceLifecycle::new(self.stats.get().payload_downloaded_bytes);
        loop {
            let complete = (async || {
                let _permit = control.begin().await?;
                if _permit.is_some()
                    && !matches!(self.stats.get().torrent_state, TrackerCommsStatsState::Live)
                {
                    control.close();
                    anyhow::bail!("Tracker monitor retired");
                }
                self.tracker_one_request_http(&tracker_url, &lifecycle, &control)
                    .await
            })
            .retry(
                ExponentialBuilder::new()
                    .without_max_times()
                    .with_jitter()
                    .with_factor(2.)
                    .with_min_delay(Duration::from_secs(10))
                    .with_max_delay(Duration::from_secs(600)),
            )
            .when(|_| !control.snapshot().closed)
            .notify(|err, retry_in| {
                debug!(?retry_in, "error calling tracker: {err:#}");
                control.failed(retry_in);
                self.stats.observe(TrackerObservation::retry(
                    &tracker_url,
                    None,
                    retry_in,
                    http_failure_message(err),
                ));
            })
            .await?;
            lifecycle.succeeded(complete.0, complete.1);
            control.finish();
            control.wait().await?;
        }
    }

    async fn tracker_one_request_http(
        &self,
        tracker_url: &Url,
        lifecycle: &AnnounceLifecycle,
        control: &TrackerControl,
    ) -> anyhow::Result<(bool, u64)> {
        self.stats
            .observe(TrackerObservation::new(tracker_url, None, "contacting"));
        let stats = self.stats.get();
        let request = tracker_comms_http::TrackerRequest {
            info_hash: &self.info_hash,
            peer_id: &self.peer_id,
            port: self.announce_port,
            uploaded: stats.uploaded_bytes,
            downloaded: stats.downloaded_bytes,
            left: stats.get_left_to_download_bytes(),
            compact: true,
            no_peer_id: false,
            event: lifecycle.event(stats.is_completed(), stats.payload_downloaded_bytes),
            ip: None,
            numwant: None,
            key: Some(self.key),
            trackerid: None,
        };

        if self.block_local_network {
            librqbit_core::network_safety::validate_url(tracker_url)?;
        }
        let mut url = tracker_url.clone();

        let mut queries = request.as_querystring();
        if let Some(url_query) = url.query() {
            queries.push_str(&format!("&{}", url_query));
        }
        url.set_query(Some(&queries));

        let response: reqwest::Response = self.reqwest_client.get(url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Tracker returned HTTP {}", response.status());
        }
        let bytes = crate::read_limited_response(response, 2 * 1024 * 1024).await?;
        if let Ok((error, _)) =
            bencode::from_bytes_with_rest::<tracker_comms_http::TrackerError>(&bytes)
        {
            anyhow::bail!("Tracker rejected announce: {}", error.failure_reason)
        };
        let response = bencode::from_bytes_with_rest::<tracker_comms_http::TrackerResponse>(&bytes)
            .map_err(|e| {
                tracing::trace!("error deserializing TrackerResponse: {e:#}");
                e.into_kind()
            })?
            .0;

        let (interval, minimum) = http_intervals(
            response.interval,
            response.min_interval,
            self.force_tracker_interval,
        )?;
        control.succeeded(interval, minimum);
        let warning = response
            .warning_message
            .as_ref()
            .map(|value| value.to_string());
        let mut observation = TrackerObservation::new(
            tracker_url,
            None,
            if warning.is_some() {
                "warning"
            } else {
                "working"
            },
        );
        observation.message = warning;
        observation.next_announce = Some(epoch_seconds().saturating_add(interval.as_secs()));
        observation.peers = Some(response.iter_peers().count() as u64);
        observation.seeds = response.complete;
        observation.leechers = response.incomplete;
        self.stats.observe(observation);
        for peer in response.iter_peers() {
            self.tx.send(peer).await?;
        }
        Ok((stats.is_completed(), stats.payload_downloaded_bytes))
    }

    async fn task_single_tracker_monitor_udp(
        &self,
        url: Url,
        client: UdpTrackerClient,
        control: Arc<TrackerControl>,
    ) -> anyhow::Result<()> {
        let _lifetime = MonitorLifetime(control.clone(), self.stats.as_ref());
        let mut lifecycles = std::collections::HashMap::<SocketAddr, AnnounceLifecycle>::new();
        if url.scheme() != "udp" {
            bail!("expected UDP scheme in {}", url);
        }
        let (host, port) = (
            url.host().context("missing host")?,
            url.port().context("missing port")?,
        );

        let mut sleep_interval: Option<Duration> = None;
        let mut prev_addrs: Option<UdpTrackerResolveResult> = None;
        loop {
            if let Some(i) = sleep_interval {
                trace!(interval=?sleep_interval, "sleeping");
                let _ = i;
                control.wait().await?;
            }

            let _permit = control.begin().await?;
            let manual = _permit.is_some();
            if manual && !matches!(self.stats.get().torrent_state, TrackerCommsStatsState::Live) {
                control.close();
                return Ok(());
            }
            self.stats
                .observe(TrackerObservation::new(&url, None, "resolving"));
            // This should retry forever until the addrs are resolved.
            let addrs = (async || {
                let resolved = udp_tracker_to_socket_addrs(host.clone(), port, client.dns.as_ref())
                    .instrument(trace_span!("resolve", ?host)).await.or_else(|err| {
                        if let Some(previous) = prev_addrs {
                            let mut observation = TrackerObservation::new(&url, None, "warning");
                            observation.message = Some("Could not refresh tracker address; using previously resolved addresses.".into());
                            self.stats.observe(observation);
                            Ok(previous)
                        } else { Err(err) }
                    })?;
                if client.block_local_network {
                    let permitted = match resolved {
                        UdpTrackerResolveResult::One(a) => librqbit_core::network_safety::is_public_destination(a.ip()),
                        UdpTrackerResolveResult::Two(a, b) => librqbit_core::network_safety::is_public_destination((*a.ip()).into()) && librqbit_core::network_safety::is_public_destination((*b.ip()).into()),
                    };
                    if !permitted { anyhow::bail!("local tracker destination requires explicit permission"); }
                }
                Ok::<_, anyhow::Error>(resolved)
            })
            .retry(
                ExponentialBuilder::new()
                    .without_max_times()
                    .with_max_delay(Duration::from_secs(60))
                    .with_jitter(),
            )
            .notify(|err, retry| {
                debug!(retry_in=?retry, "error resolving tracker: {err:#}");
                self.stats.observe(TrackerObservation::retry(
                    &url,
                    None,
                    retry,
                    "Could not resolve tracker address.".into(),
                ));
            })
            .await
            .context("this shouldn't happen: failed resolving tracker addrs")?;

            prev_addrs = Some(addrs);

            if manual && !matches!(self.stats.get().torrent_state, TrackerCommsStatsState::Live) {
                control.close();
                return Ok(());
            }
            let stats = self.stats.get();
            let complete = stats.is_completed();
            let payload = stats.payload_downloaded_bytes;
            let endpoints = match addrs {
                UdpTrackerResolveResult::One(addr) => vec![addr],
                UdpTrackerResolveResult::Two(v4, v6) => vec![v4.into(), v6.into()],
            };
            for addr in endpoints {
                lifecycles
                    .entry(addr)
                    .or_insert_with(|| AnnounceLifecycle::new(payload));
            }
            let event_for = |addr: SocketAddr| {
                lifecycles
                    .get(&addr)
                    .and_then(|l| l.event(complete, payload))
                    .or_else(|| {
                        (!lifecycles.contains_key(&addr))
                            .then_some(tracker_comms_http::TrackerRequestEvent::Started)
                    })
            };
            let results = match addrs {
                UdpTrackerResolveResult::One(addr) => vec![(
                    addr,
                    self.tracker_one_request_udp(&url, addr, &client, event_for(addr))
                        .instrument(trace_span!("udp request", ?addr))
                        .await,
                )],
                UdpTrackerResolveResult::Two(v4, v6) if manual => {
                    let r4 = self
                        .tracker_one_request_udp(&url, v4.into(), &client, event_for(v4.into()))
                        .await;
                    let r6 = self
                        .tracker_one_request_udp(&url, v6.into(), &client, event_for(v6.into()))
                        .await;
                    vec![(v4.into(), r4), (v6.into(), r6)]
                }
                UdpTrackerResolveResult::Two(v4, v6) => {
                    let (r4, r6) = tokio::join!(
                        self.tracker_one_request_udp(
                            &url,
                            v4.into(),
                            &client,
                            event_for(v4.into())
                        )
                        .instrument(trace_span!("udp request", addr=?v4)),
                        self.tracker_one_request_udp(
                            &url,
                            v6.into(),
                            &client,
                            event_for(v6.into())
                        )
                        .instrument(trace_span!("udp request", addr=?v6))
                    );
                    vec![(v4.into(), r4), (v6.into(), r6)]
                }
            };
            // Both address families share this monitor's actual next attempt.
            // Retain each outcome separately so one family cannot hide another's failure.
            let delay = results
                .iter()
                .filter_map(|(_, result)| result.as_ref().ok().map(|(delay, _)| *delay))
                .max()
                .or(sleep_interval)
                .unwrap_or(Duration::from_secs(60));
            sleep_interval = Some(delay);
            if results.iter().any(|(_, r)| r.is_ok()) {
                control.succeeded(delay, delay);
            } else {
                control.failed(delay);
            }
            for (addr, result) in results {
                match result {
                    Ok((_, mut observation)) => {
                        lifecycles
                            .entry(addr)
                            .or_default()
                            .succeeded(complete, payload);
                        observation.next_announce =
                            Some(epoch_seconds().saturating_add(delay.as_secs()));
                        self.stats.observe(observation);
                    }
                    Err(_) => self.stats.observe(TrackerObservation::retry(
                        &url,
                        Some(addr),
                        delay,
                        "UDP tracker request failed; another attempt is scheduled.".into(),
                    )),
                }
            }
            control.finish();
        }
    }

    async fn tracker_one_request_udp(
        &self,
        url: &Url,
        addr: SocketAddr,
        client: &UdpTrackerClient,
        event: Option<tracker_comms_http::TrackerRequestEvent>,
    ) -> anyhow::Result<(Duration, TrackerObservation)> {
        use tracker_comms_udp::*;

        self.stats
            .observe(TrackerObservation::new(url, Some(addr), "contacting"));
        let stats = self.stats.get();
        let request = AnnounceFields {
            info_hash: self.info_hash,
            peer_id: self.peer_id,
            downloaded: stats.downloaded_bytes,
            left: stats.get_left_to_download_bytes(),
            uploaded: stats.uploaded_bytes,
            event: match event {
                Some(tracker_comms_http::TrackerRequestEvent::Started) => EVENT_STARTED,
                Some(tracker_comms_http::TrackerRequestEvent::Completed) => EVENT_COMPLETED,
                Some(tracker_comms_http::TrackerRequestEvent::Stopped) => EVENT_STOPPED,
                None => EVENT_NONE,
            },
            key: self.key,
            port: self.announce_port,
        };

        match client.announce(addr, request).await {
            Ok(response) => {
                trace!(len = response.addrs.len(), "received announce response");
                let mut observation = TrackerObservation::new(url, Some(addr), "working");
                observation.peers = Some(response.addrs.len() as u64);
                observation.seeds = Some(response.seeders as u64);
                observation.leechers = Some(response.leechers as u64);
                self.stats.observe(observation.clone());
                for addr in response.addrs {
                    self.tx.send(addr).await.context("rx closed")?;
                }
                let sleep = response.interval.max(5);
                let sleep = Duration::from_secs(sleep as u64);
                Ok((sleep, observation))
            }
            Err(e) => {
                debug!(?addr, "error reading announce response: {e:#}");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;

    #[test]
    fn generic_transport_failure_does_not_expose_announce_credentials() {
        let error = anyhow::anyhow!(
            "failed to send request to https://user:secret@tracker.example/passkey?token=secret"
        );
        assert_eq!(
            http_failure_message(&error),
            "Tracker request failed; another attempt is scheduled."
        );
    }

    #[test]
    fn retry_keeps_address_family_identity_and_actual_schedule() {
        let url = Url::parse("udp://tracker.example:80/announce").unwrap();
        let before = epoch_seconds();
        let v4 = TrackerObservation::retry(
            &url,
            Some("127.0.0.1:80".parse().unwrap()),
            Duration::from_secs(60),
            "request failed".into(),
        );
        let v6 = TrackerObservation::retry(
            &url,
            Some("[::1]:80".parse().unwrap()),
            Duration::from_secs(60),
            "request failed".into(),
        );
        assert_ne!(v4.endpoint, v6.endpoint);
        assert_eq!(v4.state, "error");
        assert!(v4.next_announce.unwrap() >= before + 60);
        assert!(v4.next_announce.unwrap() <= epoch_seconds() + 60);
        assert_eq!(v4.peers, None);
    }
}
