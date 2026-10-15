//! Endpoint connectivity probing and latency reporting (`sshp trace`).

use std::time::Instant;

use crate::shared::{
    ALPN, AppResult, IROH_CONNECT_TIMEOUT, IrohEndpoint, NodeId, RelayUrl, iroh_error,
    timeout_error,
};

#[derive(Clone, Debug, PartialEq, Eq)]
/// Result of probing an iroh worker endpoint.
pub struct TraceReport {
    /// Ephemeral local endpoint ID used for the probe.
    pub local_id: NodeId,
    /// Target worker endpoint ID.
    pub target_id: NodeId,
    /// Elapsed milliseconds to establish the iroh QUIC connection.
    pub handshake_ms: u128,
    /// Elapsed milliseconds to open the `sshp/1` bidirectional stream.
    pub stream_open_ms: u128,
}

/// Probe connectivity and measure handshake and stream-open latency to a worker endpoint.
pub async fn trace_node(node_id: NodeId, relay: Option<RelayUrl>) -> AppResult<TraceReport> {
    let endpoint = IrohEndpoint::bind_ephemeral(relay).await?;
    let handshake_start = Instant::now();
    let connection = tokio::time::timeout(
        IROH_CONNECT_TIMEOUT,
        endpoint
            .endpoint
            .connect(endpoint.target_addr(node_id), ALPN),
    )
    .await
    .map_err(|_| timeout_error("timed out connecting to worker"))?
    .map_err(iroh_error)?;
    let handshake_ms = handshake_start.elapsed().as_millis();

    let stream_start = Instant::now();
    let (mut send, _recv) = tokio::time::timeout(IROH_CONNECT_TIMEOUT, connection.open_bi())
        .await
        .map_err(|_| timeout_error("timed out opening worker stream"))?
        .map_err(iroh_error)?;
    let stream_open_ms = stream_start.elapsed().as_millis();
    let _ = send.finish();

    Ok(TraceReport {
        local_id: endpoint.id(),
        target_id: node_id,
        handshake_ms,
        stream_open_ms,
    })
}

/// Format a [`TraceReport`] for human-readable CLI output.
pub fn format_trace_report(report: &TraceReport) -> String {
    format!(
        "local_id: {}\ntarget_id: {}\nalpn: sshp/1\nhandshake_ms: {}\nstream_open_ms: {}\nstatus: ok\n",
        report.local_id, report.target_id, report.handshake_ms, report.stream_open_ms
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_trace_report_with_expected_fields() {
        let local_id: NodeId = iroh::SecretKey::from([1; 32])
            .public()
            .to_string()
            .parse()
            .expect("local id");
        let target_id: NodeId = iroh::SecretKey::from([7; 32])
            .public()
            .to_string()
            .parse()
            .expect("target id");
        let report = TraceReport {
            local_id,
            target_id,
            handshake_ms: 42,
            stream_open_ms: 2,
        };

        let formatted = format_trace_report(&report);
        assert_eq!(
            formatted,
            format!(
                "local_id: {local_id}\ntarget_id: {target_id}\nalpn: sshp/1\nhandshake_ms: 42\nstream_open_ms: 2\nstatus: ok\n"
            )
        );
    }
}
