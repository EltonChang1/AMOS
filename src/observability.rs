use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

#[derive(Debug, Default)]
pub struct OperationalMetrics {
    task_started: AtomicU64,
    task_succeeded: AtomicU64,
    task_failed: AtomicU64,
    recovery_started: AtomicU64,
    recovery_succeeded: AtomicU64,
    recovery_failed: AtomicU64,
    task_latency_ms_total: AtomicU64,
    task_latency_samples: AtomicU64,
    task_latency_buckets: [AtomicU64; 7],
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub task_started: u64,
    pub task_succeeded: u64,
    pub task_failed: u64,
    pub recovery_started: u64,
    pub recovery_succeeded: u64,
    pub recovery_failed: u64,
    pub task_latency_ms_total: u64,
    pub task_latency_samples: u64,
    pub task_latency_bucket_bounds_ms: [u64; 6],
    pub task_latency_buckets: [u64; 7],
}

impl OperationalMetrics {
    pub fn task_started(&self) {
        self.task_started.fetch_add(1, Ordering::Relaxed);
    }

    pub fn task_finished(&self, succeeded: bool, elapsed_ms: u64) {
        let counter = if succeeded {
            &self.task_succeeded
        } else {
            &self.task_failed
        };
        counter.fetch_add(1, Ordering::Relaxed);
        self.task_latency_ms_total
            .fetch_add(elapsed_ms, Ordering::Relaxed);
        self.task_latency_samples.fetch_add(1, Ordering::Relaxed);
        let bucket = [10, 50, 100, 500, 1_000, 5_000]
            .iter()
            .position(|bound| elapsed_ms <= *bound)
            .unwrap_or(6);
        self.task_latency_buckets[bucket].fetch_add(1, Ordering::Relaxed);
    }

    pub fn recovery_started(&self) {
        self.recovery_started.fetch_add(1, Ordering::Relaxed);
    }

    pub fn recovery_finished(&self, succeeded: bool) {
        let counter = if succeeded {
            &self.recovery_succeeded
        } else {
            &self.recovery_failed
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            task_started: self.task_started.load(Ordering::Relaxed),
            task_succeeded: self.task_succeeded.load(Ordering::Relaxed),
            task_failed: self.task_failed.load(Ordering::Relaxed),
            recovery_started: self.recovery_started.load(Ordering::Relaxed),
            recovery_succeeded: self.recovery_succeeded.load(Ordering::Relaxed),
            recovery_failed: self.recovery_failed.load(Ordering::Relaxed),
            task_latency_ms_total: self.task_latency_ms_total.load(Ordering::Relaxed),
            task_latency_samples: self.task_latency_samples.load(Ordering::Relaxed),
            task_latency_bucket_bounds_ms: [10, 50, 100, 500, 1_000, 5_000],
            task_latency_buckets: std::array::from_fn(|index| {
                self.task_latency_buckets[index].load(Ordering::Relaxed)
            }),
        }
    }
}
