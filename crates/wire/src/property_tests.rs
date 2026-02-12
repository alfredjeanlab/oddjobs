// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Property tests for protocol serde roundtrips and DTO conversions.
//!
//! Covers every variant of Request, Response, and Query with minimal fixed
//! field values, plus StepRecord→StepRecordDetail outcome mapping.

use std::collections::HashMap;
use std::path::PathBuf;

use oj_core::{Event, StepOutcome, StepOutcomeKind, StepRecord};
use proptest::prelude::*;

use super::wire::{decode, encode};
use super::*;

fn s() -> String {
    String::new()
}

fn p() -> PathBuf {
    PathBuf::new()
}

fn all_requests() -> Vec<Request> {
    vec![
        Request::Ping,
        Request::Hello { version: s(), token: None },
        Request::Event { event: Event::Shutdown },
        Request::Query { query: Query::ListJobs },
        Request::Shutdown { kill: false },
        Request::Status,
        Request::AgentSend { id: s(), message: s() },
        Request::JobResume {
            id: s(),
            message: None,
            vars: HashMap::new(),
            kill: false,
            all: false,
        },
        Request::JobCancel { ids: vec![] },
        Request::JobSuspend { ids: vec![] },
        Request::RunCommand {
            project_path: p(),
            invoke_dir: p(),
            project: s(),
            command: s(),
            args: vec![],
            kwargs: HashMap::new(),
        },
        Request::WorkspaceDrop { id: s() },
        Request::WorkspaceDropFailed,
        Request::WorkspaceDropAll,
        Request::JobPrune {
            all: false,
            failed: false,
            orphans: false,
            dry_run: false,
            project: None,
        },
        Request::AgentPrune { all: false, dry_run: false },
        Request::WorkspacePrune { all: false, dry_run: false, project: None },
        Request::WorkerPrune { all: false, dry_run: false, project: None },
        Request::WorkerStart { project_path: p(), project: s(), worker: s(), all: false },
        Request::WorkerWake { worker: s(), project: s() },
        Request::WorkerStop { worker: s(), project: s(), project_path: None, all: false },
        Request::WorkerRestart { project_path: p(), project: s(), worker: s() },
        Request::WorkerResize { worker: s(), project: s(), concurrency: 0 },
        Request::CronStart { project_path: p(), project: s(), cron: s(), all: false },
        Request::CronStop { cron: s(), project: s(), project_path: None, all: false },
        Request::CronRestart { project_path: p(), project: s(), cron: s() },
        Request::CronPrune { all: false, dry_run: false },
        Request::CronOnce { project_path: p(), project: s(), cron: s() },
        Request::QueuePush {
            project_path: p(),
            project: s(),
            queue: s(),
            data: serde_json::Value::Null,
        },
        Request::QueueDrop { project_path: p(), project: s(), queue: s(), item_id: s() },
        Request::QueueRetry {
            project_path: p(),
            project: s(),
            queue: s(),
            item_ids: vec![],
            all_dead: false,
            status: None,
        },
        Request::QueueDrain { project_path: p(), project: s(), queue: s() },
        Request::QueueFail { project_path: p(), project: s(), queue: s(), item_id: s() },
        Request::QueueDone { project_path: p(), project: s(), queue: s(), item_id: s() },
        Request::QueuePrune {
            project_path: p(),
            project: s(),
            queue: s(),
            all: false,
            dry_run: false,
        },
        Request::DecisionResolve { id: s(), choices: vec![], message: None },
        Request::JobResumeAll { kill: false },
        Request::AgentResume { id: s(), kill: false, all: false },
        Request::AgentKill { id: s() },
        Request::AgentAttach { id: s(), token: None },
    ]
}

fn all_responses() -> Vec<Response> {
    vec![
        Response::Ok,
        Response::Pong,
        Response::Hello { version: s() },
        Response::ShuttingDown,
        Response::Event { accepted: false },
        Response::Jobs { jobs: vec![] },
        Response::Job { job: None },
        Response::Agents { agents: vec![] },
        Response::Agent { agent: None },
        Response::Workspaces { workspaces: vec![] },
        Response::Workspace { workspace: None },
        Response::Status { uptime_secs: 0, jobs_active: 0, orphan_count: 0 },
        Response::Error { message: s() },
        Response::JobStarted { job_id: s(), job_name: s() },
        Response::CrewStarted { crew_id: s(), agent_name: s() },
        Response::WorkspacesDropped { dropped: vec![] },
        Response::JobLogs { log_path: p(), content: s(), offset: 0 },
        Response::AgentLogs { log_path: p(), content: s(), steps: vec![], offset: 0 },
        Response::JobsPruned { pruned: vec![], skipped: 0 },
        Response::AgentsPruned { pruned: vec![], skipped: 0 },
        Response::WorkspacesPruned { pruned: vec![], skipped: 0 },
        Response::WorkersPruned { pruned: vec![], skipped: 0 },
        Response::CronsPruned { pruned: vec![], skipped: 0 },
        Response::QueuesPruned { pruned: vec![], skipped: 0 },
        Response::JobsCancelled { cancelled: vec![], already_terminal: vec![], not_found: vec![] },
        Response::JobsSuspended { suspended: vec![], already_terminal: vec![], not_found: vec![] },
        Response::WorkerStarted { worker: s() },
        Response::WorkersStarted { started: vec![], skipped: vec![] },
        Response::WorkersStopped { stopped: vec![], skipped: vec![] },
        Response::WorkerResized { worker: s(), old_concurrency: 0, new_concurrency: 0 },
        Response::CronStarted { cron: s() },
        Response::CronsStarted { started: vec![], skipped: vec![] },
        Response::CronsStopped { stopped: vec![], skipped: vec![] },
        Response::Crons { crons: vec![] },
        Response::CronLogs { log_path: p(), content: s(), offset: 0 },
        Response::QueuePushed { queue: s(), item_id: s() },
        Response::QueueDropped { queue: s(), item_id: s() },
        Response::QueueRetried {
            queue: s(),
            item_ids: vec![],
            already_retried: vec![],
            not_found: vec![],
        },
        Response::QueueDrained { queue: s(), items: vec![] },
        Response::QueueFailed { queue: s(), item_id: s() },
        Response::QueueCompleted { queue: s(), item_id: s() },
        Response::QueueItems { items: vec![] },
        Response::WorkerLogs { log_path: p(), content: s(), offset: 0 },
        Response::Workers { workers: vec![] },
        Response::Queues { queues: vec![] },
        Response::StatusOverview { uptime_secs: 0, projects: vec![], metrics_health: None },
        Response::Orphans { orphans: vec![] },
        Response::Projects { projects: vec![] },
        Response::QueueLogs { log_path: p(), content: s(), offset: 0 },
        Response::Decisions { decisions: vec![] },
        Response::Decision { decision: None },
        Response::DecisionResolved { id: s() },
        Response::AgentResumed { resumed: vec![], skipped: vec![] },
        Response::JobsResumed { resumed: vec![], skipped: vec![] },
        Response::AgentAttachReady { id: s() },
        Response::AgentAttachLocal { id: s(), socket_path: s() },
    ]
}

fn all_queries() -> Vec<Query> {
    vec![
        Query::ListJobs,
        Query::GetJob { id: s() },
        Query::ListWorkspaces,
        Query::GetWorkspace { id: s() },
        Query::GetJobLogs { id: s(), lines: 0, offset: 0 },
        Query::GetAgentLogs { id: s(), step: None, lines: 0, offset: 0 },
        Query::ListQueues { project_path: p(), project: s() },
        Query::ListQueueItems { queue: s(), project: s(), project_path: None },
        Query::GetAgent { agent_id: s() },
        Query::ListAgents { job_id: None, status: None },
        Query::GetWorkerLogs { name: s(), project: s(), lines: 0, project_path: None, offset: 0 },
        Query::ListWorkers,
        Query::ListCrons,
        Query::GetCronLogs { name: s(), project: s(), lines: 0, project_path: None, offset: 0 },
        Query::StatusOverview,
        Query::ListProjects,
        Query::ListOrphans,
        Query::DismissOrphan { id: s() },
        Query::GetQueueLogs { queue: s(), project: s(), lines: 0, offset: 0 },
        Query::ListDecisions { project: s() },
        Query::GetDecision { id: s() },
    ]
}

fn arb_step_outcome() -> impl Strategy<Value = StepOutcome> {
    prop_oneof![
        Just(StepOutcome::Running),
        Just(StepOutcome::Completed),
        ".*".prop_map(StepOutcome::Failed),
        ".*".prop_map(StepOutcome::Waiting),
    ]
}

proptest! {
    #[test]
    fn request_serde_roundtrip(req in proptest::sample::select(all_requests())) {
        let encoded = encode(&req).expect("encode");
        let decoded: Request = decode(&encoded).expect("decode");
        prop_assert_eq!(decoded, req);
    }

    #[test]
    fn response_serde_roundtrip(resp in proptest::sample::select(all_responses())) {
        let encoded = encode(&resp).expect("encode");
        let decoded: Response = decode(&encoded).expect("decode");
        prop_assert_eq!(decoded, resp);
    }

    #[test]
    fn query_serde_roundtrip(query in proptest::sample::select(all_queries())) {
        let req = Request::Query { query: query.clone() };
        let encoded = encode(&req).expect("encode");
        let decoded: Request = decode(&encoded).expect("decode");
        prop_assert_eq!(decoded, Request::Query { query });
    }

    #[test]
    fn step_record_detail_preserves_outcome(outcome in arb_step_outcome()) {
        let record = StepRecord {
            name: "test".to_string(),
            started_at_ms: 1000,
            finished_at_ms: Some(2000),
            outcome: outcome.clone(),
            agent_id: None,
            agent_name: None,
        };
        let detail = StepRecordDetail::from(&record);

        // Outcome kind is preserved
        prop_assert_eq!(detail.outcome, StepOutcomeKind::from(&outcome));

        // Detail string is preserved for variants that carry one
        match &outcome {
            StepOutcome::Failed(msg) | StepOutcome::Waiting(msg) => {
                prop_assert_eq!(detail.detail.as_ref(), Some(msg));
            }
            StepOutcome::Running | StepOutcome::Completed => {
                prop_assert_eq!(detail.detail, None);
            }
        }
    }
}
