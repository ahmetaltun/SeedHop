use std::{net::SocketAddr, sync::Arc};

/// Filters the actual addresses handed to the connector, avoiding validation/
/// connection re-resolution races. Numeric URLs are checked at request entry.
pub struct SafeDns(pub Option<crate::BoundDns>);
impl reqwest::dns::Resolve for SafeDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let bound = self.0.clone();
        Box::pin(async move {
            let addresses: Vec<SocketAddr> = match bound {
                Some(dns) => dns.lookup(name.as_str(), 0).await?,
                None => tokio::net::lookup_host((name.as_str(), 0)).await?.collect(),
            };
            let addresses: Vec<_> = addresses
                .into_iter()
                .filter(|a| librqbit_core::network_safety::is_public_destination(a.ip()))
                .collect();
            if addresses.is_empty() {
                return Err("local network destination requires explicit permission".into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

pub fn restrict_http(
    builder: reqwest::ClientBuilder,
    bound: Option<crate::BoundDns>,
) -> reqwest::ClientBuilder {
    builder
        .no_proxy()
        .dns_resolver(Arc::new(SafeDns(bound)))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many redirects");
            }
            if librqbit_core::network_safety::validate_url(attempt.url()).is_err() {
                return attempt.error("local network redirect requires explicit permission");
            }
            if attempt
                .previous()
                .last()
                .is_some_and(|previous| previous.scheme() == "https")
                && attempt.url().scheme() != "https"
            {
                return attempt.error("HTTPS downgrade rejected");
            }
            attempt.follow()
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::dns::Resolve;

    #[tokio::test]
    async fn system_resolution_cannot_return_loopback_to_the_connector() {
        let result = SafeDns(None).resolve("localhost".parse().unwrap()).await;
        assert!(result.is_err());
    }
}
