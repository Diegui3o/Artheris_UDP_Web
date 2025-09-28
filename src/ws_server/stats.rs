use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use axum::{
    extract::State,
    response::IntoResponse,
    Json,
};

use crate::ws_server::http_server::AppState;

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
    fn mark_flush_now(&self) {
        let now_ns = Instant::now()
            .duration_since(Instant::now() - Duration::from_nanos(1)) // truco p/monotonic
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

pub async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let s = &state.stats;
    let now_ns = std::time::Instant::now()
        .duration_since(std::time::Instant::now() - std::time::Duration::from_nanos(1))
        .as_nanos() as u64;
    let last = s.last_flush_instant_ns.load(Ordering::Relaxed);
    let lag_ms = if last == 0 { 0 } else { (now_ns.saturating_sub(last)) / 1_000_000 };

    Json(StatsSnapshot {
        udp_received: s.udp_received.load(Ordering::Relaxed),
        selected_kept: s.selected_kept.load(Ordering::Relaxed),
        ilp_enqueued: s.ilp_enqueued.load(Ordering::Relaxed),
        ilp_flushed: s.ilp_flushed.load(Ordering::Relaxed),
        ilp_failed: s.ilp_failed.load(Ordering::Relaxed),
        channel_depth: s.channel_depth.load(Ordering::Relaxed),
        flush_lag_ms: lag_ms,
    })
}
