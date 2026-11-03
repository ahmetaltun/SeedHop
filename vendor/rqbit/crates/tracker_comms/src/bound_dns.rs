//! DNS over TCP on an explicitly selected interface. Never uses the OS resolver.
use anyhow::{Context, bail};
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{Name, RData, RecordType},
};
use librqbit_dualstack_sockets::{BindDevice, ConnectOpts, tcp_connect};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Semaphore,
};

#[derive(Clone)]
pub struct BoundDns {
    device: BindDevice,
    server: SocketAddr,
    permits: Arc<Semaphore>,
}

impl BoundDns {
    pub fn new(device: BindDevice, server: SocketAddr) -> Self {
        Self {
            device,
            server,
            permits: Arc::new(Semaphore::new(8)),
        }
    }

    pub async fn lookup(&self, host: &str, port: u16) -> anyhow::Result<Vec<SocketAddr>> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        let mut name = Name::from_ascii(host).context("invalid DNS name")?;
        name.set_fqdn(true);
        let lookup = async {
            let _permit = self.permits.acquire().await?;
            // An interface name must still designate the originally selected index.
            let current = BindDevice::new_from_name(self.device.name())?;
            if current.index() != self.device.index() {
                bail!("selected interface changed");
            }
            let (a, aaaa) = tokio::join!(
                self.query(&name, RecordType::A),
                self.query(&name, RecordType::AAAA)
            );
            let mut result = Vec::new();
            for ips in [a, aaaa].into_iter().flatten() {
                result.extend(ips.into_iter().map(|ip| SocketAddr::new(ip, port)));
            }
            if result.is_empty() {
                bail!("selected DNS server returned no usable addresses");
            }
            Ok(result)
        };
        tokio::time::timeout(Duration::from_secs(8), lookup)
            .await
            .context("protected DNS lookup timed out")?
    }

    async fn query(&self, name: &Name, kind: RecordType) -> anyhow::Result<Vec<IpAddr>> {
        let mut request = Message::query();
        request.metadata.recursion_desired = true;
        request.add_query(Query::query(name.clone(), kind));
        let bytes = request.to_vec()?;
        let mut stream = tcp_connect(
            self.server,
            ConnectOpts {
                bind_device: Some(&self.device),
                ..Default::default()
            },
        )
        .await?;
        stream.write_u16(bytes.len().try_into()?).await?;
        stream.write_all(&bytes).await?;
        let len = usize::from(stream.read_u16().await?);
        // Bound allocation and parser work before accepting an untrusted response.
        if !(12..=16384).contains(&len) {
            bail!("invalid DNS response size");
        }
        let mut bytes = vec![0; len];
        stream.read_exact(&mut bytes).await?;
        parse_response(&bytes, &request)
    }
}

fn parse_response(bytes: &[u8], request: &Message) -> anyhow::Result<Vec<IpAddr>> {
    let response = Message::from_vec(bytes)?;
    let meta = &response.metadata;
    if meta.id != request.metadata.id
        || meta.message_type != MessageType::Response
        || meta.op_code != OpCode::Query
        || meta.truncation
        || meta.response_code != ResponseCode::NoError
        || response.queries != request.queries
    {
        bail!("invalid DNS response");
    }
    let mut owner = request.queries[0].name().clone();
    for _ in 0..8 {
        let mut addresses = Vec::new();
        let mut alias = None;
        for record in &response.answers {
            if record.name != owner {
                continue;
            }
            match &record.data {
                RData::A(ip) if request.queries[0].query_type() == RecordType::A => {
                    addresses.push(IpAddr::V4(ip.0))
                }
                RData::AAAA(ip) if request.queries[0].query_type() == RecordType::AAAA => {
                    addresses.push(IpAddr::V6(ip.0))
                }
                RData::CNAME(name) => alias = Some(name.0.clone()),
                _ => (),
            }
        }
        if !addresses.is_empty() {
            return Ok(addresses);
        }
        match alias {
            Some(name) => owner = name,
            None => return Ok(Vec::new()),
        }
    }
    bail!("DNS alias chain is too long")
}

impl reqwest::dns::Resolve for BoundDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let this = self.clone();
        Box::pin(async move {
            let addresses = this.lookup(name.as_str(), 0).await?;
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::{Record, rdata::A};

    fn request() -> Message {
        let mut message = Message::query();
        message.add_query(Query::query(
            Name::from_ascii("seedhop.example.").unwrap(),
            RecordType::A,
        ));
        message
    }
    fn response(request: &Message) -> Message {
        let mut response = Message::response(request.metadata.id, OpCode::Query);
        response.queries = request.queries.clone();
        response.add_answer(Record::from_rdata(
            request.queries[0].name().clone(),
            60,
            RData::A(A("203.0.113.7".parse().unwrap())),
        ));
        response
    }
    #[test]
    fn validates_transaction_question_and_answer_owner() {
        let request = request();
        let mut answer = response(&request);
        assert_eq!(
            parse_response(&answer.to_vec().unwrap(), &request).unwrap(),
            vec!["203.0.113.7".parse::<IpAddr>().unwrap()]
        );
        answer.metadata.id ^= 1;
        assert!(parse_response(&answer.to_vec().unwrap(), &request).is_err());
        answer = response(&request);
        answer.queries[0] =
            Query::query(Name::from_ascii("other.example.").unwrap(), RecordType::A);
        assert!(parse_response(&answer.to_vec().unwrap(), &request).is_err());
        answer = response(&request);
        answer.answers[0].name = Name::from_ascii("other.example.").unwrap();
        assert!(
            parse_response(&answer.to_vec().unwrap(), &request)
                .unwrap()
                .is_empty()
        );
        for bytes in [&[][..], &[0u8; 12][..], &[255u8; 512][..]] {
            assert!(parse_response(bytes, &request).is_err());
        }
    }

    #[tokio::test]
    async fn resolves_only_through_explicit_bound_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let len = stream.read_u16().await.unwrap();
                let mut bytes = vec![0; usize::from(len)];
                stream.read_exact(&mut bytes).await.unwrap();
                let request = Message::from_vec(&bytes).unwrap();
                assert_eq!(request.queries[0].name().to_ascii(), "seedhop.example.");
                let response = response(&request).to_vec().unwrap();
                stream.write_u16(response.len() as u16).await.unwrap();
                stream.write_all(&response).await.unwrap();
            }
        });
        #[cfg(target_os = "macos")]
        let name = "lo0";
        #[cfg(not(target_os = "macos"))]
        let name = "lo";
        let dns = BoundDns::new(BindDevice::new_from_name(name).unwrap(), server);
        let addresses = dns.lookup("seedhop.example", 6881).await.unwrap();
        assert_eq!(addresses, vec!["203.0.113.7:6881".parse().unwrap()]);
        task.await.unwrap();
        // The fake name is not resolvable by public DNS. Closing its explicit server
        // must fail, never retry through the system's DNS service.
        assert!(dns.lookup("seedhop.example", 6881).await.is_err());
    }
}
