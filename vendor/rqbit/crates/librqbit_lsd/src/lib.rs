use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    str::{FromStr, from_utf8},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use anyhow::Context;
use futures::Stream;
use librqbit_core::{Id20, spawn_utils::spawn_with_cancel};
use librqbit_dualstack_sockets::{BindDevice, MulticastUdpSocket};
use parking_lot::RwLock;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio_util::sync::CancellationToken;
use tracing::{debug, debug_span, trace};

const LSD_PORT: u16 = 6771;
const LSD_IPV4: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 192, 152, 143), LSD_PORT);
const LSD_IPV6: SocketAddrV6 = SocketAddrV6::new(
    Ipv6Addr::new(0xff15, 0, 0, 0, 0, 0, 0xefc0, 0x988f),
    LSD_PORT,
    0,
    0,
);

const RATE_LIMIT_PERIOD: Duration = Duration::from_secs(1);

#[derive(Default)]
struct RateLimiter {
    last_reply: AtomicU64,
}

impl RateLimiter {
    fn check(&self) -> Option<()> {
        // If we can't get system time for some reason, just disable rate limit
        let now = match SystemTime::UNIX_EPOCH.elapsed() {
            Ok(t) => t.as_secs(),
            _ => return Some(()),
        };

        let last = self.last_reply.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= RATE_LIMIT_PERIOD.as_secs()
            && self
                .last_reply
                .compare_exchange_weak(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            return Some(());
        }
        None
    }
}

struct Subscriber {
    tx: Sender<SocketAddr>,
    port: Option<u16>,
}

#[derive(Default)]
struct Announce {
    subscribers: HashMap<u64, Subscriber>,
    announcers: HashMap<u16, CancellationToken>,
    last_reply_ipv4: RateLimiter,
    last_reply_ipv6: RateLimiter,
}

struct LocalServiceDiscoveryInner {
    socket: MulticastUdpSocket,
    cookie: u32,
    cancel_token: CancellationToken,
    receivers: RwLock<HashMap<Id20, Announce>>,
    next_subscription: AtomicU64,
}

#[derive(Clone)]
pub struct LocalServiceDiscovery {
    inner: Arc<LocalServiceDiscoveryInner>,
}

#[derive(Default)]
pub struct LocalServiceDiscoveryOptions<'a> {
    pub cancel_token: CancellationToken,
    pub cookie: Option<u32>,
    pub bind_device: Option<&'a BindDevice>,
}

impl LocalServiceDiscovery {
    pub async fn new(opts: LocalServiceDiscoveryOptions<'_>) -> anyhow::Result<Self> {
        let socket = MulticastUdpSocket::new(
            (Ipv6Addr::UNSPECIFIED, LSD_PORT).into(),
            LSD_IPV4,
            LSD_IPV6,
            None,
            opts.bind_device,
        )
        .await
        .context("error binding LSD socket")?;
        let cookie = opts.cookie.unwrap_or_else(rand::random);
        let lsd = Self {
            inner: Arc::new(LocalServiceDiscoveryInner {
                socket,
                cookie,
                cancel_token: opts.cancel_token.clone(),
                receivers: Default::default(),
                next_subscription: AtomicU64::new(0),
            }),
        };

        spawn_with_cancel(
            debug_span!("lsd"),
            "lsd",
            opts.cancel_token,
            lsd.clone().task_monitor_recv(),
        );

        Ok(lsd)
    }

    fn gen_announce_msg(&self, info_hash: Id20, port: u16, is_v6: bool) -> String {
        let host: SocketAddr = if is_v6 {
            LSD_IPV6.into()
        } else {
            LSD_IPV4.into()
        };
        let cookie = self.inner.cookie;
        let info_hash = info_hash.as_string();
        format!(
            "BT-SEARCH * HTTP/1.1\r
Host: {host}\r
Port: {port}\r
Infohash: {info_hash}\r
cookie: {cookie}\r
\r
\r
"
        )
    }

    async fn recv_and_process_one(&self, buf: &mut [u8]) -> anyhow::Result<()> {
        macro_rules! return_if_none {
            ($e:expr) => {
                return_if_none!($e, ())
            };
            ($e:expr, $if_err:expr) => {
                match $e {
                    Some(e) => e,
                    None => {
                        $if_err;
                        return Ok(());
                    }
                }
            };
        }

        let mut headers = [httparse::EMPTY_HEADER; 16];

        let (sz, addr) = self.inner.socket.recv_from(buf).await?;
        let buf = bstr::BStr::new(&buf[..sz]);

        let bts = return_if_none!(
            try_parse_bt_search(buf, &mut headers)
                .inspect_err(|e| trace!(?buf, ?addr, "error parsing message: {e:#}"))
                .ok()
        );

        trace!(?addr, ?bts, "received");

        if bts.our_cookie == Some(self.inner.cookie) {
            trace!(?bts, "ignoring our own message");
            return Ok(());
        }

        let (announce_port, announce_cancel) = {
            let g = self.inner.receivers.read();
            let announce = return_if_none!(g.get(&bts.hash));
            let mut addr = addr;
            addr.set_port(bts.port);

            // Discovery must not grow an unbounded queue behind a resolver
            // whose connection slots are occupied. A later announcement may
            // offer the endpoint again once the consumer has capacity.
            for subscriber in announce.subscribers.values() {
                match subscriber.tx.try_send(addr) {
                    Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
                }
            }

            let (announce_port, announce_cancel) =
                return_if_none!(announce.announcers.iter().next());

            let rl = if addr.is_ipv4() {
                &announce.last_reply_ipv4
            } else {
                &announce.last_reply_ipv6
            };

            return_if_none!(rl.check(), trace!(?addr, ?bts, "replying rate-limited"));

            (*announce_port, announce_cancel.clone())
        };

        let mopts = return_if_none!(
            self.inner.socket.find_mcast_opts_for_replying_to(&addr),
            debug!(?addr, "couldn't find where to reply")
        );

        let reply = self.gen_announce_msg(bts.hash, announce_port, addr.is_ipv6());
        let result = tokio::select! {
            biased;
            _ = announce_cancel.cancelled() => return Ok(()),
            result = self.inner.socket.send_multicast_msg(reply.as_bytes(), &mopts) => result,
        };
        if let Err(e) = result {
            trace!(?addr, ?reply, ?mopts, "error sending reply: {e:#}");
        } else {
            trace!(?addr, ?reply, ?mopts, "sent reply");
        }
        Ok(())
    }

    async fn task_monitor_recv(self) -> anyhow::Result<()> {
        let mut buf = [0u8; 4096];

        loop {
            self.recv_and_process_one(&mut buf).await?;
        }
    }

    async fn task_announce_periodically(
        self,
        info_hash: Id20,
        port: u16,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
        let mut interval = tokio::time::interval(Duration::from_secs(60 * 5));
        loop {
            interval.tick().await;
            if cancel.is_cancelled() {
                return Ok(());
            }

            self.inner
                .socket
                .try_send_mcast_everywhere(&|mopts| {
                    (!cancel.is_cancelled()).then(|| {
                        self.gen_announce_msg(info_hash, port, mopts.mcast_addr().is_ipv6())
                    })
                })
                .await;
        }
    }

    pub fn announce(
        &self,
        info_hash: Id20,
        announce_port: Option<u16>,
    ) -> impl Stream<Item = SocketAddr> + Send + Sync + 'static {
        // 1. Periodically announce the torrent.
        // 2. Stream back the results from received messages.

        let (tx, rx) = channel::<SocketAddr>(1024);

        struct AddrStream {
            info_hash: Id20,
            subscription: u64,
            rx: Receiver<SocketAddr>,
            lsd: LocalServiceDiscovery,
        }

        impl Stream for AddrStream {
            type Item = SocketAddr;

            fn poll_next(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Option<Self::Item>> {
                self.rx.poll_recv(cx)
            }
        }

        impl Drop for AddrStream {
            fn drop(&mut self) {
                let mut receivers = self.lsd.inner.receivers.write();
                if let Some(announce) = receivers.get_mut(&self.info_hash) {
                    if let Some(Subscriber {
                        port: Some(port), ..
                    }) = announce.subscribers.remove(&self.subscription)
                        && !announce
                            .subscribers
                            .values()
                            .any(|subscriber| subscriber.port == Some(port))
                        && let Some(cancel) = announce.announcers.remove(&port)
                    {
                        cancel.cancel();
                    }
                    if announce.subscribers.is_empty() {
                        receivers.remove(&self.info_hash);
                    }
                }
            }
        }

        let subscription = self.inner.next_subscription.fetch_add(1, Ordering::Relaxed);
        let new_worker = {
            let mut receivers = self.inner.receivers.write();
            let announce = receivers.entry(info_hash).or_default();
            announce.subscribers.insert(
                subscription,
                Subscriber {
                    tx,
                    port: announce_port,
                },
            );
            announce_port.and_then(|port| {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    announce.announcers.entry(port)
                {
                    let cancel = self.inner.cancel_token.child_token();
                    entry.insert(cancel.clone());
                    Some((port, cancel))
                } else {
                    None
                }
            })
        };

        if let Some((port, cancel)) = new_worker {
            spawn_with_cancel(
                debug_span!(parent: None, "lsd-announce", ?info_hash, port),
                "lsd-announce",
                cancel.clone(),
                self.clone()
                    .task_announce_periodically(info_hash, port, cancel),
            );
        }

        AddrStream {
            info_hash,
            subscription,
            rx,
            lsd: self.clone(),
        }
    }
}

#[derive(Debug)]
struct BtSearchAnnounceMessage {
    hash: Id20,
    our_cookie: Option<u32>,
    #[allow(unused)]
    host: SocketAddr,
    port: u16,
}

fn try_parse_bt_search<'a: 'h, 'h>(
    buf: &'a [u8],
    headers: &'a mut [httparse::Header<'h>],
) -> anyhow::Result<BtSearchAnnounceMessage> {
    let mut req = httparse::Request::new(headers);
    req.parse(buf).context("error parsing request")?;

    match req.method {
        Some("BT-SEARCH") => {
            let mut host = None;
            let mut port = None;
            let mut infohash = None;
            let mut our_cookie = None;

            for header in req.headers.iter() {
                if header.name.eq_ignore_ascii_case("host") {
                    host = Some(
                        from_utf8(header.value)
                            .context("invalid utf-8 in host header")?
                            .parse()
                            .context("invalid IP in host header")?,
                    );
                } else if header.name.eq_ignore_ascii_case("port") {
                    port = Some(atoi::atoi::<u16>(header.value).context("port is not a number")?)
                } else if header.name.eq_ignore_ascii_case("infohash") {
                    infohash = Some(
                        Id20::from_str(from_utf8(header.value).context("infohash isn't utf-8")?)
                            .context("invalid infohash header")?,
                    );
                } else if header.name.eq_ignore_ascii_case("cookie") {
                    our_cookie = atoi::atoi::<u32>(header.value);
                }
            }

            match (host, port, infohash) {
                (Some(host), Some(port), Some(hash)) => Ok(BtSearchAnnounceMessage {
                    hash,
                    our_cookie,
                    host,
                    port,
                }),
                _ => anyhow::bail!("not all of host, man and st are set"),
            }
        }
        _ => anyhow::bail!("expecting BT-SEARCH"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    async fn discovery() -> (LocalServiceDiscovery, CancellationToken) {
        let stop = CancellationToken::new();
        let lsd = LocalServiceDiscovery::new(LocalServiceDiscoveryOptions {
            cancel_token: stop.clone(),
            cookie: Some(12345),
            ..Default::default()
        })
        .await
        .unwrap();
        (lsd, stop)
    }

    async fn offer(socket: &tokio::net::UdpSocket, hash: Id20, port: u16) {
        let message = format!(
            "BT-SEARCH * HTTP/1.1\r\nHost: [ff15::efc0:988f]:6771\r\nPort: {port}\r\nInfohash: {}\r\ncookie: 54321\r\n\r\n\r\n",
            hash.as_string()
        );
        socket
            .send_to(message.as_bytes(), "[::1]:6771")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn subscribers_share_announcements_and_drop_only_their_own_registration() {
        let (lsd, stop) = discovery().await;
        let hash = Id20::new([7; 20]);
        let mut first = lsd.announce(hash, None);
        let mut second = lsd.announce(hash, None);
        let sender = tokio::net::UdpSocket::bind("[::1]:0").await.unwrap();
        offer(&sender, hash, 32123).await;
        let expected: SocketAddr = "[::1]:32123".parse().unwrap();
        for stream in [&mut first, &mut second] {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), stream.next())
                    .await
                    .unwrap(),
                Some(expected)
            );
        }
        drop(first);
        offer(&sender, hash, 32124).await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), second.next())
                .await
                .unwrap(),
            Some("[::1]:32124".parse().unwrap())
        );
        drop(second);
        assert!(lsd.inner.receivers.read().is_empty());
        stop.cancel();
    }

    #[tokio::test]
    async fn last_announcing_subscriber_drop_stops_its_periodic_task() {
        let (lsd, stop) = discovery().await;
        let baseline = Arc::strong_count(&lsd.inner);
        let hash = Id20::new([8; 20]);
        let listener = lsd.announce(hash, None);
        let first = lsd.announce(hash, Some(32125));
        let second = lsd.announce(hash, Some(32125));
        // One worker for the shared port, plus the three subscriber streams.
        assert_eq!(Arc::strong_count(&lsd.inner), baseline + 4);
        drop(first);
        // The surviving announcing subscriber still owns that worker.
        assert_eq!(Arc::strong_count(&lsd.inner), baseline + 3);
        drop(second);
        tokio::time::timeout(Duration::from_secs(1), async {
            while Arc::strong_count(&lsd.inner) != baseline + 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("periodic announcement task outlived its last owner");
        drop(listener);
        assert!(lsd.inner.receivers.read().is_empty());
        stop.cancel();
    }

    #[tokio::test]
    async fn a_full_subscriber_queue_does_not_block_another_subscriber() {
        let (lsd, stop) = discovery().await;
        let hash = Id20::new([9; 20]);
        let slow = lsd.announce(hash, None);
        let mut fast = lsd.announce(hash, None);
        let sender = tokio::net::UdpSocket::bind("[::1]:0").await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            for port in 32000..33100 {
                offer(&sender, hash, port).await;
                assert_eq!(
                    fast.next().await,
                    Some(SocketAddr::new(Ipv6Addr::LOCALHOST.into(), port))
                );
            }
        })
        .await
        .expect("one full discovery queue blocked another subscriber");
        drop(slow);
        drop(fast);
        stop.cancel();
    }
}
