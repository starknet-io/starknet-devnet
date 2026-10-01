#![allow(clippy::expect_used)]
use lazy_static::lazy_static;
use prometheus::{HistogramVec, IntCounterVec, Opts, Registry};

lazy_static! {
    /// Includes cancelled attempts that were still waiting for lifecycle access.
    pub static ref LIFECYCLE_LOCK_WAIT: HistogramVec = HistogramVec::new(
        prometheus::HistogramOpts::new(
            "devnet_lifecycle_lock_wait_seconds",
            "Time waiting for lifecycle access, including cancelled attempts"
        ).buckets(vec![0.00001, 0.0001, 0.001, 0.01, 0.1, 1.0, 10.0, 60.0]),
        &["mode"]
    ).expect("Failed to create LIFECYCLE_LOCK_WAIT histogram");

    pub static ref LIFECYCLE_LOCK_HOLD: HistogramVec = HistogramVec::new(
        prometheus::HistogramOpts::new(
            "devnet_lifecycle_lock_hold_seconds",
            "Time holding lifecycle access"
        ).buckets(vec![0.00001, 0.0001, 0.001, 0.01, 0.1, 1.0, 10.0, 60.0]),
        &["mode"]
    ).expect("Failed to create LIFECYCLE_LOCK_HOLD histogram");

    /// Histogram tracking RPC call duration in seconds
    pub static ref RPC_CALL_DURATION: HistogramVec = HistogramVec::new(
        prometheus::HistogramOpts::new(
            "rpc_call_duration_seconds",
            "Duration of RPC calls in seconds"
        )
        .buckets(vec![
            0.00005, 0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.015, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0
        ]),
        &["method"]
    )
    .expect("Failed to create RPC_CALL_DURATION histogram");

    /// Counter tracking total RPC calls by method and status
    pub static ref RPC_CALL_COUNT: IntCounterVec = IntCounterVec::new(
        Opts::new("rpc_call_count", "Total number of RPC calls"),
        &["method", "status"]
    )
    .expect("Failed to create RPC_CALL_COUNT counter");
}

/// Register all server metrics with the provided registry
pub fn register_metrics(registry: &Registry) -> Result<(), prometheus::Error> {
    registry.register(Box::new(LIFECYCLE_LOCK_WAIT.clone()))?;
    registry.register(Box::new(LIFECYCLE_LOCK_HOLD.clone()))?;
    registry.register(Box::new(RPC_CALL_DURATION.clone()))?;
    registry.register(Box::new(RPC_CALL_COUNT.clone()))?;
    Ok(())
}
