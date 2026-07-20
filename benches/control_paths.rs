use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use amos::{
    AmosRuntime, RuntimeConfig,
    api::demo_identities,
    domain::{AuditEvent, Authority, MemoryObject, MemoryType, new_id},
    memory::{MemoryService, RetrieveQuery},
    policy::PolicyEngine,
    seed,
    store::Store,
};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use tempfile::TempDir;

fn percentile(samples: &mut [Duration], percentile: usize) -> Duration {
    samples.sort_unstable();
    let index = samples.len().saturating_sub(1).saturating_mul(percentile) / 100;
    samples[index]
}

fn sample<E, F>(iterations: usize, mut operation: F) -> Result<Vec<Duration>, E>
where
    F: FnMut(usize) -> Result<(), E>,
{
    let mut samples = Vec::with_capacity(iterations);
    for iteration in 0..iterations {
        let started = Instant::now();
        operation(iteration)?;
        samples.push(started.elapsed());
    }
    Ok(samples)
}

fn summary(
    workload: &str,
    samples: &[Duration],
    p95_threshold: Duration,
) -> Result<Value, Box<dyn std::error::Error>> {
    let p50 = percentile(&mut samples.to_vec(), 50);
    let p95 = percentile(&mut samples.to_vec(), 95);
    let p99 = percentile(&mut samples.to_vec(), 99);
    if p95 > p95_threshold {
        return Err(format!(
            "{workload} p95 {p95:?} exceeded capacity threshold {p95_threshold:?}"
        )
        .into());
    }
    Ok(json!({
        "workload": workload,
        "iterations": samples.len(),
        "p50_micros": p50.as_micros(),
        "p95_micros": p95.as_micros(),
        "p99_micros": p99.as_micros(),
        "p95_threshold_micros": p95_threshold.as_micros(),
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = TempDir::new()?;
    let store = Store::open(root.path().join("capacity.sqlite"))?;
    let identity = &demo_identities()["analyst_001"];
    let item_count = std::env::var("AMOS_BENCH_MEMORY_ITEMS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(if cfg!(debug_assertions) { 500 } else { 10_000 });
    for index in 0..item_count {
        let mut object = MemoryObject::new(
            &identity.tenant_id,
            format!("benchmark:memory:{index}"),
            MemoryType::Document,
            format!("payment benchmark evidence item {index}"),
            json!({"role":"benchmark","index":index,"topic":"payment failure"}),
            "benchmark",
            format!("v{index}"),
            Authority::SystemObserved,
        )?;
        object.permissions = BTreeSet::from(["analytics".into(), "payments".into()]);
        store.write_memory(&object)?;
    }
    let service = MemoryService::new(store.clone(), PolicyEngine);
    let retrieval_samples = sample(100, |_| {
        let result = service.retrieve(
            identity,
            &RetrieveQuery {
                task_text: "payment failure evidence".into(),
                required_types: BTreeSet::from([MemoryType::Document]),
                time_start: Utc::now() - ChronoDuration::days(1),
                time_end: Utc::now() + ChronoDuration::days(1),
                max_items: 20,
            },
        )?;
        assert_eq!(result.items.len(), 20);
        Ok::<_, amos::AmosError>(())
    })?;

    let control_iterations = std::env::var("AMOS_BENCH_CONTROL_ITERATIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(if cfg!(debug_assertions) { 5 } else { 10 });
    let config = RuntimeConfig::demo(root.path());
    let control_store = Store::open(&config.control_db)?;
    seed::seed_demo(&control_store, &config.warehouse_db)?;
    let runtime = AmosRuntime::open(config)?;
    let tokio = tokio::runtime::Builder::new_multi_thread().build()?;
    let identities = demo_identities();
    let analyst = &identities["analyst_001"];
    let reviewer = &identities["reviewer_001"];
    let task_text = "Why did payment failure rate increase over the last six hours, and should we update the executive dashboard?";
    let reference_run = tokio.block_on(runtime.run_task(
        analyst,
        task_text.into(),
        "benchmark-reference-task".into(),
    ))?;
    let valid_sql = reference_run.plan.steps[0]
        .parameters
        .get("sql")
        .and_then(Value::as_str)
        .ok_or("reference plan did not contain SQL")?
        .to_string();

    let verification_samples = sample(control_iterations, |_| {
        runtime.preflight_sql(analyst, task_text, valid_sql.clone())?;
        Ok::<_, amos::AmosError>(())
    })?;
    let commit_samples = sample(control_iterations, |iteration| {
        runtime.store.append_audit(&AuditEvent {
            event_id: format!("audit_benchmark_{iteration}"),
            tenant_id: analyst.tenant_id.clone(),
            actor_id: analyst.subject_id.clone(),
            action: "benchmark.commit".into(),
            target_type: "benchmark".into(),
            target_id: format!("sample_{iteration}"),
            request_id: None,
            atxn_id: None,
            outcome: "pass".into(),
            policy_epoch: analyst.policy_epoch,
            details: json!({"iteration":iteration}),
            created_at: Utc::now(),
        })
    })?;
    let job_samples = sample(control_iterations, |iteration| {
        let job = runtime.scheduler.enqueue(
            "benchmark_tenant",
            "benchmark.noop",
            json!({"iteration":iteration}),
            format!("benchmark-job-{iteration}"),
        )?;
        let acquired = runtime
            .scheduler
            .acquire("benchmark_tenant", "benchmark-worker", 30)?
            .ok_or_else(|| amos::AmosError::NotFound(job.job_id.clone()))?;
        runtime
            .scheduler
            .complete(acquired.clone(), acquired.fencing_token)?;
        Ok::<_, amos::AmosError>(())
    })?;
    let data_object = reference_run
        .manifest
        .required_role_coverage
        .get("data_snapshot")
        .and_then(|ids| ids.first())
        .ok_or("reference manifest did not cover data_snapshot")?
        .clone();
    let invalidation_samples = sample(control_iterations, |iteration| {
        runtime.revalidate_artifact(reviewer, &reference_run.artifact.artifact_id)?;
        runtime.evidence.invalidate_memory_with_key(
            &analyst.tenant_id,
            &data_object,
            "benchmark invalidation",
            &format!("benchmark-invalidation-{iteration}"),
        )?;
        Ok::<_, amos::AmosError>(())
    })?;
    let task_samples = sample(control_iterations, |iteration| {
        tokio.block_on(runtime.run_task(
            analyst,
            task_text.into(),
            format!("benchmark-task-{iteration}"),
        ))?;
        Ok::<_, amos::AmosError>(())
    })?;
    let replay_samples = sample(control_iterations, |iteration| {
        tokio.block_on(runtime.replay_async(
            analyst,
            reference_run.artifact.artifact_id.clone(),
            format!("benchmark-replay-{iteration}"),
        ))?;
        Ok::<_, amos::AmosError>(())
    })?;

    let fast_threshold = if cfg!(debug_assertions) {
        Duration::from_secs(2)
    } else {
        Duration::from_millis(250)
    };
    let workflow_threshold = if cfg!(debug_assertions) {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(2)
    };
    let workloads = vec![
        summary(
            "bounded_memory_retrieval",
            &retrieval_samples,
            fast_threshold,
        )?,
        summary(
            "sql_verification_preflight",
            &verification_samples,
            fast_threshold,
        )?,
        summary("durable_control_commit", &commit_samples, fast_threshold)?,
        summary("job_lease_and_complete", &job_samples, fast_threshold)?,
        summary(
            "claim_invalidation_page",
            &invalidation_samples,
            fast_threshold,
        )?,
        summary("governed_task_total", &task_samples, workflow_threshold)?,
        summary(
            "persisted_computational_replay",
            &replay_samples,
            workflow_threshold,
        )?,
    ];
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "memory_items": item_count,
            "result_k": 20,
            "control_iterations": control_iterations,
            "build_profile": if cfg!(debug_assertions) {"debug"} else {"release"},
            "workloads": workloads,
            "run_id": new_id("bench"),
        }))?
    );
    Ok(())
}
