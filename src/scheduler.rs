use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Duration, Utc};
use serde_json::Value;

use crate::{
    Result,
    domain::{Job, JobState, OutboxEvent},
    error::AmosError,
    store::Store,
};

pub trait OutboxDestination: Send + Sync {
    fn deliver(&self, event: &OutboxEvent) -> Result<()>;
}

#[derive(Clone)]
pub struct OutboxDispatcher<D> {
    store: Store,
    destination: D,
}

impl<D: OutboxDestination> OutboxDispatcher<D> {
    pub fn new(store: Store, destination: D) -> Self {
        Self { store, destination }
    }

    pub fn dispatch_one(
        &self,
        tenant: &str,
        dispatcher_id: &str,
        lease_seconds: i64,
    ) -> Result<Option<OutboxEvent>> {
        if lease_seconds <= 0 {
            return Err(AmosError::Validation(
                "outbox lease duration must be positive".into(),
            ));
        }
        let now = Utc::now();
        let Some(event) = self.store.acquire_outbox(
            tenant,
            dispatcher_id,
            now,
            now + Duration::seconds(lease_seconds),
        )?
        else {
            return Ok(None);
        };
        let fence = event.fencing_token;
        let delivery_error = self
            .destination
            .deliver(&event)
            .err()
            .map(|error| error.to_string());
        Ok(Some(self.store.finish_outbox(
            &event,
            fence,
            dispatcher_id,
            Utc::now(),
            delivery_error,
        )?))
    }

    pub fn dispatch_batch(
        &self,
        tenant: &str,
        dispatcher_id: &str,
        lease_seconds: i64,
        max_events: usize,
        shutdown: &AtomicBool,
    ) -> Result<Vec<OutboxEvent>> {
        if max_events == 0 {
            return Err(AmosError::Validation(
                "outbox batch must allow at least one event".into(),
            ));
        }
        let mut dispatched = Vec::new();
        while dispatched.len() < max_events && !shutdown.load(Ordering::Acquire) {
            let Some(event) = self.dispatch_one(tenant, dispatcher_id, lease_seconds)? else {
                break;
            };
            dispatched.push(event);
        }
        Ok(dispatched)
    }
}

#[derive(Clone)]
pub struct Scheduler {
    store: Store,
}
impl Scheduler {
    pub fn new(store: Store) -> Self {
        Self { store }
    }
    pub fn enqueue(
        &self,
        tenant: &str,
        job_type: &str,
        payload: Value,
        idempotency_key: String,
    ) -> Result<Job> {
        self.store
            .enqueue_job(&Job::ready(tenant, job_type, payload, idempotency_key, 5))
    }
    pub fn acquire(&self, tenant: &str, worker: &str, lease_seconds: i64) -> Result<Option<Job>> {
        if lease_seconds <= 0 {
            return Err(AmosError::Validation(
                "job lease duration must be positive".into(),
            ));
        }
        let now = Utc::now();
        self.store
            .acquire_job(tenant, worker, now, now + Duration::seconds(lease_seconds))
    }

    pub fn renew(&self, mut job: Job, fence: u64, lease_seconds: i64) -> Result<Job> {
        if lease_seconds <= 0 {
            return Err(AmosError::Validation(
                "job lease duration must be positive".into(),
            ));
        }
        let owner = active_lease_owner(&job, fence)?;
        let now = Utc::now();
        job.lease_expires_at = Some(now + Duration::seconds(lease_seconds));
        self.store.renew_job_lease(&job, fence, &owner, now)?;
        Ok(job)
    }

    pub fn complete(&self, mut job: Job, fence: u64) -> Result<Job> {
        let owner = active_lease_owner(&job, fence)?;
        let now = Utc::now();
        job.state = JobState::Complete;
        job.lease_owner = None;
        job.lease_expires_at = None;
        self.store.finish_job(&job, fence, &owner, now)?;
        Ok(job)
    }

    pub fn fail(&self, mut job: Job, fence: u64, reason: String) -> Result<Job> {
        if reason.trim().is_empty() {
            return Err(AmosError::Validation(
                "job failure requires a reason".into(),
            ));
        }
        let owner = active_lease_owner(&job, fence)?;
        let now = Utc::now();
        if job.attempt >= job.max_attempts {
            job.state = JobState::DeadLetter;
            job.dead_letter_reason = Some(reason);
        } else {
            job.state = JobState::RetryWait;
            job.next_run_at = now + Duration::seconds(2_i64.pow(job.attempt.min(8)));
        }
        job.lease_owner = None;
        job.lease_expires_at = None;
        self.store.finish_job(&job, fence, &owner, now)?;
        Ok(job)
    }
}

fn active_lease_owner(job: &Job, fence: u64) -> Result<String> {
    if job.state != JobState::Running || job.fencing_token != fence {
        return Err(AmosError::Conflict("stale job fence".into()));
    }
    job.lease_owner
        .as_ref()
        .filter(|owner| !owner.trim().is_empty())
        .cloned()
        .ok_or_else(|| AmosError::Conflict("job has no active lease owner".into()))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingDestination {
        delivered_keys: Arc<Mutex<Vec<String>>>,
    }

    impl OutboxDestination for RecordingDestination {
        fn deliver(&self, event: &OutboxEvent) -> Result<()> {
            self.delivered_keys
                .lock()
                .map_err(|_| AmosError::Execution("recording destination lock poisoned".into()))?
                .push(event.idempotency_key.clone());
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct FailingDestination;

    impl OutboxDestination for FailingDestination {
        fn deliver(&self, _event: &OutboxEvent) -> Result<()> {
            Err(AmosError::Connector("destination unavailable".into()))
        }
    }

    #[test]
    fn dispatcher_delivers_a_bounded_batch_and_marks_completion() {
        let store = Store::in_memory().unwrap();
        store
            .enqueue_job(&Job::ready(
                "tenant",
                "test",
                json!({"value":1}),
                "dispatch-fixture",
                1,
            ))
            .unwrap();
        let destination = RecordingDestination::default();
        let delivered_keys = destination.delivered_keys.clone();
        let dispatcher = OutboxDispatcher::new(store.clone(), destination);
        let shutdown = AtomicBool::new(false);
        let delivered = dispatcher
            .dispatch_batch("tenant", "dispatcher", 30, 10, &shutdown)
            .unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].state, crate::domain::OutboxState::Delivered);
        assert_eq!(delivered_keys.lock().unwrap().len(), 1);
        assert!(
            store
                .list_outbox("tenant", 10)
                .unwrap()
                .iter()
                .all(|event| event.completed_at.is_some())
        );

        shutdown.store(true, Ordering::Release);
        assert!(
            dispatcher
                .dispatch_batch("tenant", "dispatcher", 30, 10, &shutdown)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn dispatcher_retries_delivery_errors_and_dead_letters_at_the_attempt_limit() {
        let store = Store::in_memory().unwrap();
        store
            .enqueue_job(&Job::ready(
                "tenant",
                "test",
                json!({"value":1}),
                "failing-dispatch-fixture",
                1,
            ))
            .unwrap();
        let dispatcher = OutboxDispatcher::new(store.clone(), FailingDestination);
        let mut event = dispatcher
            .dispatch_one("tenant", "dispatcher", 30)
            .unwrap()
            .unwrap();
        assert_eq!(event.state, crate::domain::OutboxState::RetryWait);
        assert!(
            event
                .last_error
                .as_deref()
                .is_some_and(|error| { error.contains("destination unavailable") })
        );

        while event.state != crate::domain::OutboxState::DeadLetter {
            let now = event.next_attempt_at.unwrap() + Duration::milliseconds(1);
            let acquired = store
                .acquire_outbox("tenant", "dispatcher", now, now + Duration::seconds(30))
                .unwrap()
                .unwrap();
            event = store
                .finish_outbox(
                    &acquired,
                    acquired.fencing_token,
                    "dispatcher",
                    now,
                    Some("destination unavailable".into()),
                )
                .unwrap();
        }
        assert_eq!(event.attempt, event.max_attempts);
        assert!(event.completed_at.is_some());
        assert!(event.lease_owner.is_none());
    }
}
