// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Generic lifecycle helpers for workers and crons.

use std::path::Path;

use crate::storage::MaterializedState;
use oj_core::Event;

use super::mutations::{emit, PruneFlags};
use super::{ConnectionError, ListenCtx};
use crate::protocol::Response;

/// (started_names, skipped_with_reason)
pub(super) type StartAllResult = (Vec<String>, Vec<(String, String)>);

/// Trait for resources with stop/restart lifecycle.
pub(super) trait Stoppable {
    const TYPE_NAME: &'static str;
    const APPLY_STOP: bool;
    fn exists(state: &MaterializedState, scoped_name: &str) -> bool;
    fn running_names(state: &MaterializedState, project: &str) -> Vec<String>;
    fn stopped_event(name: &str, project: &str) -> Event;
}

fn stop_and_apply<T: Stoppable>(
    ctx: &ListenCtx,
    name: &str,
    ns: &str,
) -> Result<(), ConnectionError> {
    let event = T::stopped_event(name, ns);
    emit(&ctx.event_bus, event.clone())?;
    if T::APPLY_STOP {
        ctx.state.lock().apply_event(&event);
    }
    Ok(())
}

/// Stop a single resource, or all running resources when `all` is true.
pub(super) fn handle_stop<T: Stoppable>(
    ctx: &ListenCtx,
    name: &str,
    project: &str,
    all: bool,
    suggest_fn: impl FnOnce() -> String,
    batch_response: impl FnOnce(Vec<String>, Vec<(String, String)>) -> Response,
) -> Result<Response, ConnectionError> {
    if all {
        let names = {
            let s = ctx.state.lock();
            T::running_names(&s, project)
        };
        let mut stopped = Vec::new();
        let mut skipped = Vec::new();
        for n in names {
            match stop_and_apply::<T>(ctx, &n, project) {
                Ok(()) => stopped.push(n),
                Err(e) => skipped.push((n, e.to_string())),
            }
        }
        return Ok(batch_response(stopped, skipped));
    }
    let scoped = oj_core::scoped_name(project, name);
    if !T::exists(&ctx.state.lock(), &scoped) {
        let hint = suggest_fn();
        return Ok(Response::Error {
            message: format!("unknown {}: {}{}", T::TYPE_NAME, name, hint),
        });
    }
    stop_and_apply::<T>(ctx, name, project)?;
    Ok(Response::Ok)
}

/// Restart: stop (if running), then start fresh.
pub(super) fn handle_restart<T: Stoppable>(
    ctx: &ListenCtx,
    project: &str,
    name: &str,
    start_fn: impl FnOnce() -> Result<Response, ConnectionError>,
) -> Result<Response, ConnectionError> {
    let scoped = oj_core::scoped_name(project, name);
    if T::exists(&ctx.state.lock(), &scoped) {
        let event = T::stopped_event(name, project);
        emit(&ctx.event_bus, event.clone())?;
        ctx.state.lock().apply_event(&event);
    }
    start_fn()
}

/// Start all resources defined in runbooks.
pub(super) fn handle_start_all(
    ctx: &ListenCtx,
    project_path: &Path,
    project: &str,
    collect_names: impl FnOnce(&Path) -> Vec<String>,
    start_one: impl Fn(&str) -> Result<Response, ConnectionError>,
    extract_name: impl Fn(&Response) -> Option<String>,
) -> Result<StartAllResult, ConnectionError> {
    let root = super::resolve_effective_project_path(project_path, project, &ctx.state);
    let mut started = Vec::new();
    let mut skipped = Vec::new();
    for name in collect_names(&root.join(".oj/runbooks")) {
        match start_one(&name) {
            Ok(ref resp) => match extract_name(resp) {
                Some(n) => started.push(n),
                None => match resp {
                    Response::Error { message } => skipped.push((name, message.clone())),
                    _ => skipped.push((name, "unexpected response".to_string())),
                },
            },
            Err(e) => skipped.push((name, e.to_string())),
        }
    }
    Ok((started, skipped))
}

/// Prune stopped resources: collect, optionally delete, return response.
pub(super) fn handle_prune<E>(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
    collect: impl FnOnce(&MaterializedState, Option<&str>) -> (Vec<E>, usize),
    delete: impl Fn(&E) -> Event,
    response: impl FnOnce(Vec<E>, usize) -> Response,
) -> Result<Response, ConnectionError> {
    let (to_prune, skipped) = collect(&ctx.state.lock(), flags.project);
    if !flags.dry_run {
        for entry in &to_prune {
            emit(&ctx.event_bus, delete(entry))?;
        }
    }
    Ok(response(to_prune, skipped))
}
