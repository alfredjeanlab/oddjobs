// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session and timer event tests

use super::*;
use oj_core::TimerId;

#[tokio::test]
async fn timer_event_for_non_session_monitor_ignored() {
    let ctx = setup().await;
    let _pipeline_id = create_pipeline(&ctx).await;

    // Timer with unknown prefix should be ignored
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::new("other:timer"),
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn custom_event_unknown_is_ignored() {
    let ctx = setup().await;
    let _pipeline_id = create_pipeline(&ctx).await;

    let result = ctx.runtime.handle_event(Event::Custom).await.unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn liveness_timer_for_nonexistent_pipeline_is_noop() {
    let ctx = setup().await;

    // Liveness timer for a nonexistent pipeline should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::new("liveness:nonexistent"),
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn exit_deferred_timer_for_nonexistent_pipeline_is_noop() {
    let ctx = setup().await;

    // Deferred exit timer for a nonexistent pipeline should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::new("exit-deferred:nonexistent"),
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn get_pipeline_returns_none_for_nonexistent() {
    let ctx = setup().await;

    let pipeline = ctx.runtime.get_pipeline("nonexistent");
    assert!(pipeline.is_none());
}

#[tokio::test]
async fn get_pipeline_returns_pipeline_by_prefix() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Get by full ID
    let p1 = ctx.runtime.get_pipeline(&pipeline_id);
    assert!(p1.is_some());

    // Get by prefix (pipe-1 -> pipe)
    let prefix = &pipeline_id[..4];
    let p2 = ctx.runtime.get_pipeline(prefix);
    assert!(p2.is_some());
    assert_eq!(p2.unwrap().id, pipeline_id);
}

#[tokio::test]
async fn pipelines_returns_all_pipelines() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    let pipelines = ctx.runtime.pipelines();
    assert_eq!(pipelines.len(), 1);
    assert!(pipelines.contains_key(&pipeline_id));
}
