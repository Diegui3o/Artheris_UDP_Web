use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant};

#[derive(Default, Debug)]
pub struct IngestStats {
    pub udp_received: AtomicU64,
    pub selected_kept: AtomicU64,
    pub ilp_enqueued: AtomicU64,
    pub ilp_flushed: AtomicU64,
    pub ilp_failed: AtomicU64,
    pub last_flush_instant_ns: AtomicU64,
    pub channel_depth: AtomicU64,
}

impl IngestStats {
    pub fn mark_flush_now(&self) {
        let now_ns = Instant::now()
            .elapsed()
            .as_nanos() as u64;

        self.last_flush_instant_ns.store(now_ns, Ordering::Relaxed);
    }
}

#[derive(serde::Serialize)]
pub struct StatsSnapshot {
    pub udp_received: u64,
    pub selected_kept: u64,
    pub ilp_enqueued: u64,
    pub ilp_flushed: u64,
    pub ilp_failed: u64,
    pub channel_depth: u64,
    pub flush_lag_ms: u64,
}