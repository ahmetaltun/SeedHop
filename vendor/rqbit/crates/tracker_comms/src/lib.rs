mod bound_dns;
mod reannounce;
pub use bound_dns::BoundDns;
mod safe_dns;
pub use safe_dns::restrict_http;

/// Bound remote response bodies before buffering or parsing them.
pub async fn read_limited_response(
    mut response: reqwest::Response,
    limit: usize,
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "response exceeds size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            chunk.len() <= limit.saturating_sub(bytes.len()),
            "response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub use reannounce::{ReannounceDisposition, ReannounceSnapshot, TrackerControl};
mod tracker_comms;
mod tracker_comms_http;
mod tracker_comms_udp;

pub use tracker_comms::*;
pub use tracker_comms_udp::UdpTrackerClient;

#[cfg(test)]
mod reannounce_tests;
