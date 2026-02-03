// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Protocol unit tests

use std::collections::HashMap;

use super::*;
use oj_core::{Event, PipelineId};

#[test]
fn encode_decode_roundtrip_request() {
    let request = Request::Event {
        event: Event::CommandRun {
            pipeline_id: PipelineId::new("pipe-1"),
            pipeline_name: "build".to_string(),
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
        pipelines_active: 5,
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
        query: Query::GetPipeline {
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
fn pipeline_summary_serialization() {
    let summary = PipelineSummary {
        id: "pipe-1".to_string(),
        name: "build feature".to_string(),
        kind: "build".to_string(),
        step: "Execute".to_string(),
        step_status: "Running".to_string(),
        created_at_ms: 1700000000000,
        updated_at_ms: 1700000001000,
        namespace: String::new(),
        retry_count: 0,
    };

    let response = Response::Pipelines {
        pipelines: vec![summary.clone()],
    };

    let encoded = encode(&response).expect("encode failed");
    let decoded: Response = decode(&encoded).expect("decode failed");

    match decoded {
        Response::Pipelines { pipelines } => {
            assert_eq!(pipelines.len(), 1);
            assert_eq!(pipelines[0], summary);
        }
        _ => panic!("Expected Pipelines response"),
    }
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
            pipeline_id: Some("pipe-".to_string()),
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
            pipeline_id: None,
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
            pipeline_id: "pipe-abc".to_string(),
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
fn agent_summary_pipeline_id_default() {
    // Verify serde default works for pipeline_id (backward compat)
    let json = r#"{"step_name":"build","agent_id":"a1","status":"running","files_read":0,"files_written":0,"commands_run":0,"exit_reason":null}"#;
    let summary: AgentSummary = serde_json::from_str(json).expect("deserialize failed");
    assert_eq!(summary.pipeline_id, "");
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
            pipeline_id: "pipe-abc".to_string(),
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
            queue_type: "persisted".to_string(),
            item_count: 5,
            workers: vec!["fixer".to_string()],
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
            pipeline_id: "pipe-orphan".to_string(),
            project: "myproject".to_string(),
            kind: "deploy".to_string(),
            name: "deploy-staging".to_string(),
            current_step: "build".to_string(),
            step_status: "Running".to_string(),
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
        pipelines_active: 2,
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
    let json = r#"{"type":"Status","uptime_secs":60,"pipelines_active":1,"sessions_active":0}"#;
    let decoded: Response = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Response::Status { orphan_count, .. } => assert_eq!(orphan_count, 0),
        _ => panic!("Expected Status response"),
    }
}

#[test]
fn encode_decode_roundtrip_pipeline_prune_with_failed() {
    let request = Request::PipelinePrune {
        all: false,
        failed: true,
        orphans: false,
        dry_run: true,
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn encode_decode_roundtrip_pipeline_prune_with_orphans() {
    let request = Request::PipelinePrune {
        all: false,
        failed: false,
        orphans: true,
        dry_run: false,
    };

    let encoded = encode(&request).expect("encode failed");
    let decoded: Request = decode(&encoded).expect("decode failed");

    assert_eq!(request, decoded);
}

#[test]
fn pipeline_prune_failed_defaults_to_false() {
    // Backward compatibility: old PipelinePrune without `failed` should deserialize
    let json = r#"{"type":"PipelinePrune","all":false,"dry_run":true}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::PipelinePrune {
            all,
            failed,
            orphans,
            dry_run,
        } => {
            assert!(!all);
            assert!(!failed);
            assert!(!orphans);
            assert!(dry_run);
        }
        _ => panic!("Expected PipelinePrune request"),
    }
}

#[test]
fn pipeline_prune_orphans_defaults_to_false() {
    // Backward compatibility: old PipelinePrune without `orphans` should deserialize
    let json = r#"{"type":"PipelinePrune","all":true,"failed":false,"dry_run":false}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::PipelinePrune {
            all,
            failed,
            orphans,
            dry_run,
        } => {
            assert!(all);
            assert!(!failed);
            assert!(!orphans);
            assert!(!dry_run);
        }
        _ => panic!("Expected PipelinePrune request"),
    }
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
