// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Protocol unit tests

use std::collections::HashMap;

use super::*;
use oj_core::{Event, JobId, StepOutcome, StepOutcomeKind, StepRecord, StepStatusKind};

#[test]
fn encode_decode_roundtrip_request() {
    let request = Request::Event {
        event: Event::CommandRun {
            job_id: JobId::new("pipe-1"),
            job_name: "build".to_string(),
            project_root: std::path::PathBuf::from("/test/project"),
            invoke_dir: std::path::PathBuf::from("/test/project"),
            command: "build".to_string(),
            namespace: String::new(),
            args: HashMap::from([("name".to_string(), "test".to_string())]),
        },
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_roundtrip_response() {
    let response = Response::Status {
        uptime_secs: 3600,
        jobs_active: 5,
        sessions_active: 3,
        orphan_count: 0,
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[test]
fn encode_decode_query() {
    let request = Request::Query {
        query: Query::GetJob {
            id: "pipe-123".to_string(),
        },
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_returns_json_without_length_prefix() {
    let response = Response::Ok;
    let encoded = encode(&response).expect("encode failed");

    // encode() returns raw JSON, no length prefix
    let json_str = std::str::from_utf8(&encoded).expect("should be valid UTF-8");
    assert!(
        json_str.starts_with('{'),
        "should be JSON object: {}",
        json_str
    );
}

#[test]
fn job_summary_serialization() {
    let summary = JobSummary {
        id: "pipe-1".to_string(),
        name: "build feature".to_string(),
        kind: "build".to_string(),
        step: "Execute".to_string(),
        step_status: StepStatusKind::Running,
        created_at_ms: 1700000000000,
        updated_at_ms: 1700000001000,
        namespace: String::new(),
        retry_count: 0,
    };

    let response = Response::Jobs {
        jobs: vec![summary.clone()],
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    match decoded {
        Response::Jobs { jobs } => {
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0], summary);
        }
        _ => panic!("Expected Jobs response"),
    }
}

#[test]
fn encode_decode_roundtrip_session_kill() {
    let request = Request::SessionKill {
        id: "oj-ses-abc123".to_string(),
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_roundtrip_peek_session() {
    let request = Request::PeekSession {
        session_id: "ses-abc123".to_string(),
        with_color: true,
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_roundtrip_session_peek() {
    let response = Response::SessionPeek {
        output: "$ cargo build\n   Compiling odd jobs v0.1.0\n".to_string(),
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[tokio::test]
async fn read_write_message_roundtrip() {
    let original = b"hello world";

    let mut buffer = Vec::new();
    write_message(&mut buffer, original)
        .await
        .expect("write failed");

    // write_message adds 4-byte length prefix
    assert_eq!(buffer.len(), 4 + original.len());

    let mut cursor = std::io::Cursor::new(buffer);
    let read_back = read_message(&mut cursor).await.expect("read failed");

    assert_eq!(read_back, original);
}

#[test]
fn encode_decode_list_workers_query() {
    let request = Request::Query {
        query: Query::ListWorkers,
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_workers_response() {
    let response = Response::Workers {
        workers: vec![WorkerSummary {
            name: "fixer".to_string(),
            queue: "bugs".to_string(),
            status: "running".to_string(),
            active: 2,
            concurrency: 3,
            namespace: String::new(),
            updated_at_ms: 0,
        }],
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[test]
fn encode_decode_list_agents_query() {
    let request = Request::Query {
        query: Query::ListAgents {
            job_id: Some("pipe-".to_string()),
            status: Some("running".to_string()),
        },
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_list_agents_query_no_filters() {
    let request = Request::Query {
        query: Query::ListAgents {
            job_id: None,
            status: None,
        },
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_agents_response() {
    let response = Response::Agents {
        agents: vec![AgentSummary {
            job_id: "pipe-abc".to_string(),
            step_name: "build".to_string(),
            agent_id: "agent-123".to_string(),
            agent_name: Some("claude".to_string()),
            namespace: Some("myproject".to_string()),
            status: "running".to_string(),
            files_read: 10,
            files_written: 3,
            commands_run: 5,
            exit_reason: None,
            updated_at_ms: 1700000000000,
        }],
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[test]
fn encode_decode_agents_response_empty() {
    let response = Response::Agents { agents: vec![] };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[test]
fn agent_summary_job_id_default() {
    // Verify serde default works for job_id (backward compat)
    let json = r#"{"step_name":"build","agent_id":"a1","status":"running","files_read":0,"files_written":0,"commands_run":0,"exit_reason":null}"#;
    let summary: AgentSummary = serde_json::from_str(json).expect("deserialize failed");
    assert_eq!(summary.job_id, "");
    assert_eq!(summary.step_name, "build");
}

#[test]
fn encode_decode_roundtrip_agent_send() {
    let request = Request::AgentSend {
        agent_id: "abc-123".to_string(),
        message: "hello agent".to_string(),
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_roundtrip_agent_prune_request() {
    let request = Request::AgentPrune {
        all: true,
        dry_run: false,
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_roundtrip_agents_pruned_response() {
    let response = Response::AgentsPruned {
        pruned: vec![AgentEntry {
            agent_id: "agent-123".to_string(),
            job_id: "pipe-abc".to_string(),
            step_name: "build".to_string(),
        }],
        skipped: 2,
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[test]
fn encode_decode_list_queues_query() {
    let request = Request::Query {
        query: Query::ListQueues {
            project_root: std::path::PathBuf::from("/test/project"),
            namespace: "myproject".to_string(),
        },
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_queues_response() {
    let response = Response::Queues {
        queues: vec![QueueSummary {
            name: "tasks".to_string(),
            namespace: "myproject".to_string(),
            queue_type: "persisted".to_string(),
            item_count: 5,
            workers: vec!["fixer".to_string()],
            last_poll_count: Some(3),
            last_polled_at_ms: Some(1700000000000),
        }],
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[test]
fn encode_decode_queues_response_empty() {
    let response = Response::Queues { queues: vec![] };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[test]
fn encode_decode_list_orphans_query() {
    let request = Request::Query {
        query: Query::ListOrphans,
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_dismiss_orphan_query() {
    let request = Request::Query {
        query: Query::DismissOrphan {
            id: "pipe-abc".to_string(),
        },
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_orphans_response() {
    let response = Response::Orphans {
        orphans: vec![OrphanSummary {
            job_id: "pipe-orphan".to_string(),
            project: "myproject".to_string(),
            kind: "deploy".to_string(),
            name: "deploy-staging".to_string(),
            current_step: "build".to_string(),
            step_status: StepStatusKind::Running,
            workspace_root: Some(std::path::PathBuf::from("/tmp/ws")),
            agents: vec![OrphanAgent {
                agent_id: "pipe-orphan-build".to_string(),
                session_name: Some("oj-pipe-orphan-build".to_string()),
                log_path: std::path::PathBuf::from("/state/logs/agent/pipe-orphan-build.log"),
            }],
            updated_at: "2026-01-30T08:14:09Z".to_string(),
        }],
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[test]
fn encode_decode_status_with_orphans() {
    let response = Response::Status {
        uptime_secs: 100,
        jobs_active: 2,
        sessions_active: 1,
        orphan_count: 3,
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    assert_eq!(response, decoded);
}

#[test]
fn status_orphan_count_defaults_to_zero() {
    // Test backward compatibility: old Status without orphan_count should deserialize
    let json = r#"{"type":"Status","uptime_secs":60,"jobs_active":1,"sessions_active":0}"#;
    let decoded: Response = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Response::Status { orphan_count, .. } => assert_eq!(orphan_count, 0),
        _ => panic!("Expected Status response"),
    }
}

#[test]
fn encode_decode_roundtrip_job_prune_with_failed() {
    let request = Request::JobPrune {
        all: false,
        failed: true,
        orphans: false,
        dry_run: true,
        namespace: None,
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_roundtrip_job_prune_with_orphans() {
    let request = Request::JobPrune {
        all: false,
        failed: false,
        orphans: true,
        dry_run: false,
        namespace: None,
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn job_prune_failed_defaults_to_false() {
    // Backward compatibility: old JobPrune without `failed` should deserialize
    let json = r#"{"type":"JobPrune","all":false,"dry_run":true}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::JobPrune {
            all,
            failed,
            orphans,
            dry_run,
            namespace,
        } => {
            assert!(!all);
            assert!(!failed);
            assert!(!orphans);
            assert!(dry_run);
            assert!(namespace.is_none());
        }
        _ => panic!("Expected JobPrune request"),
    }
}

#[test]
fn job_prune_orphans_defaults_to_false() {
    // Backward compatibility: old JobPrune without `orphans` should deserialize
    let json = r#"{"type":"JobPrune","all":true,"failed":false,"dry_run":false}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::JobPrune {
            all,
            failed,
            orphans,
            dry_run,
            namespace,
        } => {
            assert!(all);
            assert!(!failed);
            assert!(!orphans);
            assert!(!dry_run);
            assert!(namespace.is_none());
        }
        _ => panic!("Expected JobPrune request"),
    }
}

#[test]
fn encode_decode_roundtrip_job_prune_with_namespace() {
    let request = Request::JobPrune {
        all: true,
        failed: false,
        orphans: false,
        dry_run: false,
        namespace: Some("my-project".to_string()),
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn job_prune_namespace_defaults_to_none() {
    // Backward compatibility: old JobPrune without `namespace` should deserialize
    let json = r#"{"type":"JobPrune","all":true,"failed":false,"orphans":false,"dry_run":false}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::JobPrune { namespace, .. } => {
            assert!(namespace.is_none());
        }
        _ => panic!("Expected JobPrune request"),
    }
}

#[test]
fn encode_decode_roundtrip_workspace_prune_with_namespace() {
    let request = Request::WorkspacePrune {
        all: true,
        dry_run: false,
        namespace: Some("my-project".to_string()),
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn workspace_prune_namespace_defaults_to_none() {
    // Backward compatibility: old WorkspacePrune without `namespace` should deserialize
    let json = r#"{"type":"WorkspacePrune","all":false,"dry_run":true}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::WorkspacePrune { namespace, .. } => {
            assert!(namespace.is_none());
        }
        _ => panic!("Expected WorkspacePrune request"),
    }
}

#[test]
fn encode_decode_roundtrip_worker_restart() {
    let request = Request::WorkerRestart {
        project_root: std::path::PathBuf::from("/test/project"),
        namespace: "myproject".to_string(),
        worker_name: "fixer".to_string(),
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_roundtrip_cron_restart() {
    let request = Request::CronRestart {
        project_root: std::path::PathBuf::from("/test/project"),
        namespace: "myproject".to_string(),
        cron_name: "nightly".to_string(),
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[tokio::test]
async fn write_message_adds_length_prefix() {
    let data = b"test data";

    let mut buffer = Vec::new();
    write_message(&mut buffer, data)
        .await
        .expect("write failed");

    // First 4 bytes are the length prefix
    let len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;

    // Length should match the data size
    assert_eq!(len, data.len());
    assert_eq!(&buffer[4..], data);
}

#[test]
fn step_record_detail_from_running() {
    let record = StepRecord {
        name: "build".to_string(),
        started_at_ms: 1000,
        finished_at_ms: None,
        outcome: StepOutcome::Running,
        agent_id: Some("agent-1".to_string()),
        agent_name: Some("coder".to_string()),
    };
    let detail = StepRecordDetail::from(&record);
    assert_eq!(detail.name, "build");
    assert_eq!(detail.started_at_ms, 1000);
    assert_eq!(detail.finished_at_ms, None);
    assert_eq!(detail.outcome, StepOutcomeKind::Running);
    assert_eq!(detail.detail, None);
    assert_eq!(detail.agent_id, Some("agent-1".to_string()));
    assert_eq!(detail.agent_name, Some("coder".to_string()));
}

#[test]
fn step_record_detail_from_completed() {
    let record = StepRecord {
        name: "test".to_string(),
        started_at_ms: 2000,
        finished_at_ms: Some(3000),
        outcome: StepOutcome::Completed,
        agent_id: None,
        agent_name: None,
    };
    let detail = StepRecordDetail::from(&record);
    assert_eq!(detail.outcome, StepOutcomeKind::Completed);
    assert_eq!(detail.finished_at_ms, Some(3000));
    assert_eq!(detail.detail, None);
}

#[test]
fn step_record_detail_from_failed() {
    let record = StepRecord {
        name: "deploy".to_string(),
        started_at_ms: 4000,
        finished_at_ms: Some(5000),
        outcome: StepOutcome::Failed("compilation error".to_string()),
        agent_id: Some("agent-2".to_string()),
        agent_name: None,
    };
    let detail = StepRecordDetail::from(&record);
    assert_eq!(detail.outcome, StepOutcomeKind::Failed);
    assert_eq!(detail.detail, Some("compilation error".to_string()));
}

#[test]
fn step_record_detail_from_waiting() {
    let record = StepRecord {
        name: "gate".to_string(),
        started_at_ms: 6000,
        finished_at_ms: None,
        outcome: StepOutcome::Waiting("gate check failed".to_string()),
        agent_id: None,
        agent_name: None,
    };
    let detail = StepRecordDetail::from(&record);
    assert_eq!(detail.outcome, StepOutcomeKind::Waiting);
    assert_eq!(detail.detail, Some("gate check failed".to_string()));
}
