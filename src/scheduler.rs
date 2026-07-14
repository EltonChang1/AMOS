use chrono::{Duration, Utc};
use serde_json::Value;

use crate::{
    Result,
    domain::{Job, JobState, new_id},
    error::AmosError,
    store::Store,
};

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
        self.store.enqueue_job(&Job {
            job_id: new_id("job"),
            tenant_id: tenant.into(),
            job_type: job_type.into(),
            payload,
            idempotency_key,
            state: JobState::Ready,
            attempt: 0,
            max_attempts: 5,
            fencing_token: 0,
            lease_owner: None,
            lease_expires_at: None,
            next_run_at: Utc::now(),
            dead_letter_reason: None,
        })
    }
    pub fn acquire(&self, tenant: &str, worker: &str, lease_seconds: i64) -> Result<Option<Job>> {
        self.store.acquire_job(
            tenant,
            worker,
            Utc::now() + Duration::seconds(lease_seconds),
        )
    }
    pub fn complete(&self, mut job: Job, fence: u64) -> Result<Job> {
        if job.fencing_token != fence {
            return Err(AmosError::Conflict("stale job fence".into()));
        }
        job.state = JobState::Complete;
        job.lease_owner = None;
        job.lease_expires_at = None;
        self.store.update_job_with_fence(&job, fence)?;
        Ok(job)
    }
    pub fn fail(&self, mut job: Job, fence: u64, reason: String) -> Result<Job> {
        if job.fencing_token != fence {
            return Err(AmosError::Conflict("stale job fence".into()));
        }
        if job.attempt >= job.max_attempts {
            job.state = JobState::DeadLetter;
            job.dead_letter_reason = Some(reason);
        } else {
            job.state = JobState::RetryWait;
            job.next_run_at = Utc::now() + Duration::seconds(2_i64.pow(job.attempt.min(8)));
        }
        job.lease_owner = None;
        job.lease_expires_at = None;
        self.store.update_job_with_fence(&job, fence)?;
        Ok(job)
    }
}
