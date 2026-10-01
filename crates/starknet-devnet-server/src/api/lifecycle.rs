use prometheus::HistogramTimer;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::metrics::{LIFECYCLE_LOCK_HOLD, LIFECYCLE_LOCK_WAIT};

/// Coordinates state, journals, and ordered notification enqueueing across RPC transports.
/// Timers distinguish lock contention from the work performed while holding the lock.
#[derive(Default)]
pub struct Lifecycle {
    lock: RwLock<()>,
}

pub struct LifecycleGuard<G> {
    guard: Option<G>,
    timer: Option<HistogramTimer>,
}

impl<G> Drop for LifecycleGuard<G> {
    fn drop(&mut self) {
        drop(self.guard.take());
        if let Some(timer) = self.timer.take() {
            timer.observe_duration();
        }
    }
}

impl Lifecycle {
    pub async fn read(&self) -> LifecycleGuard<RwLockReadGuard<'_, ()>> {
        let waiting = LIFECYCLE_LOCK_WAIT.with_label_values(&["read"]).start_timer();
        let guard = self.lock.read().await;
        waiting.observe_duration();
        LifecycleGuard {
            guard: Some(guard),
            timer: Some(LIFECYCLE_LOCK_HOLD.with_label_values(&["read"]).start_timer()),
        }
    }

    pub async fn write(&self) -> LifecycleGuard<RwLockWriteGuard<'_, ()>> {
        let waiting = LIFECYCLE_LOCK_WAIT.with_label_values(&["write"]).start_timer();
        let guard = self.lock.write().await;
        waiting.observe_duration();
        LifecycleGuard {
            guard: Some(guard),
            timer: Some(LIFECYCLE_LOCK_HOLD.with_label_values(&["write"]).start_timer()),
        }
    }
}
