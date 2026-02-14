// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session and timer event tests

use super::*;
use oj_core::TimerId;

#[tokio::test]
async fn timer_event_for_non_session_monitor_ignored() {
    let ctx = setup().await;
    let _job_id = create_job(&ctx).await;

    // Timer with unknown prefix should be ignored
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart { id: TimerId::from_string("other:timer") })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn custom_event_unknown_is_ignored() {
    let ctx = setup().await;
    let _job_id = create_job(&ctx).await;

    let result = ctx.runtime.handle_event(Event::Custom).await.unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn liveness_timer_for_nonexistent_job_is_noop() {
    let ctx = setup().await;

    // Liveness timer for a nonexistent job should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart { id: TimerId::from_string("liveness:nonexistent") })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn exit_deferred_timer_for_nonexistent_job_is_noop() {
    let ctx = setup().await;

    // Deferred exit timer for a nonexistent job should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart { id: TimerId::from_string("exit-deferred:nonexistent") })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn get_job_returns_none_for_nonexistent() {
    let ctx = setup().await;

    let job = ctx.runtime.get_job("nonexistent");
    assert!(job.is_none());
}

#[tokio::test]
async fn get_job_returns_job_by_prefix() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Get by full ID
    let p1 = ctx.runtime.get_job(&job_id);
    assert!(p1.is_some());

    // Get by prefix (job-1 -> pipe)
    let prefix = &job_id[..4];
    let p2 = ctx.runtime.get_job(prefix);
    assert!(p2.is_some());
    assert_eq!(p2.unwrap().id, job_id);
}

#[tokio::test]
async fn jobs_returns_all_jobs() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    let jobs = ctx.runtime.jobs();
    assert_eq!(jobs.len(), 1);
    assert!(jobs.contains_key(&job_id));
}
