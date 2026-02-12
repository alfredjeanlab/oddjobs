// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

mod entity_tests;
mod job_tests;
mod project_tests;
mod status_tests;

use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

use crate::storage::MaterializedState;

use oj_core::Breadcrumb;

use crate::listener::test_fixtures::{
    make_breadcrumb_full as make_breadcrumb, make_cron, make_decision, make_job_full as make_job,
    make_queue_item, make_worker,
};
use crate::listener::ListenCtx;
use crate::protocol::{Query, Response};

use super::handle_query as real_handle_query;

/// Wrapper that constructs a ListenCtx from individual params for test convenience.
fn handle_query(
    query: Query,
    state: &Arc<Mutex<MaterializedState>>,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    logs_path: &std::path::Path,
    start_time: Instant,
) -> Response {
    let ctx = ListenCtx {
        event_bus: {
            let wal = crate::storage::Wal::open(&logs_path.join("__query_test.wal"), 0).unwrap();
            let (bus, _reader) = crate::event_bus::EventBus::new(wal);
            bus
        },
        state: Arc::clone(state),
        orphans: Arc::clone(orphans),
        metrics_health: Arc::new(Mutex::new(Default::default())),
        state_dir: logs_path.to_path_buf(),
        logs_path: logs_path.to_path_buf(),
        start_time,
        shutdown: Arc::new(tokio::sync::Notify::new()),
        auth_token: None,
        agent_adapter: None,
    };
    real_handle_query(&ctx, query)
}

fn empty_state() -> Arc<Mutex<MaterializedState>> {
    Arc::new(Mutex::new(MaterializedState::default()))
}

fn empty_orphans() -> Arc<Mutex<Vec<Breadcrumb>>> {
    Arc::new(Mutex::new(Vec::new()))
}
