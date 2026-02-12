// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::fs;
use std::io::Write;

use oj_adapters::agent::UsageData;

use super::*;

/// Minimal no-op adapter for tests that only exercise write/rotate.
#[derive(Clone)]
struct StubAdapter;

#[async_trait::async_trait]
impl oj_adapters::agent::AgentAdapter for StubAdapter {
    async fn spawn(
        &self,
        _: oj_adapters::agent::AgentConfig,
        _: tokio::sync::mpsc::Sender<oj_core::Event>,
    ) -> Result<oj_adapters::agent::AgentHandle, oj_adapters::agent::AgentAdapterError> {
        unimplemented!()
    }
    async fn send(
        &self,
        _: &oj_core::AgentId,
        _: &str,
    ) -> Result<(), oj_adapters::agent::AgentAdapterError> {
        unimplemented!()
    }
    async fn kill(
        &self,
        _: &oj_core::AgentId,
    ) -> Result<(), oj_adapters::agent::AgentAdapterError> {
        unimplemented!()
    }
    async fn reconnect(
        &self,
        _: oj_adapters::agent::AgentReconnectConfig,
        _: tokio::sync::mpsc::Sender<oj_core::Event>,
    ) -> Result<oj_adapters::agent::AgentHandle, oj_adapters::agent::AgentAdapterError> {
        unimplemented!()
    }
    async fn get_state(
        &self,
        _: &oj_core::AgentId,
    ) -> Result<oj_core::AgentState, oj_adapters::agent::AgentAdapterError> {
        unimplemented!()
    }
    async fn last_message(&self, _: &oj_core::AgentId) -> Option<String> {
        None
    }
    async fn resolve_stop(&self, _: &oj_core::AgentId) {}
    async fn is_alive(&self, _: &oj_core::AgentId) -> bool {
        false
    }
    async fn capture_output(
        &self,
        _: &oj_core::AgentId,
        _: u32,
    ) -> Result<String, oj_adapters::agent::AgentAdapterError> {
        unimplemented!()
    }
    async fn fetch_transcript(
        &self,
        _: &oj_core::AgentId,
    ) -> Result<String, oj_adapters::agent::AgentAdapterError> {
        unimplemented!()
    }
    async fn fetch_usage(&self, _: &oj_core::AgentId) -> Option<UsageData> {
        None
    }
}

#[test]
fn write_records_produces_valid_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let metrics_dir = dir.path().join("metrics");

    let collector = UsageMetricsCollector {
        state: Arc::new(Mutex::new(MaterializedState::default())),
        agents: StubAdapter,
        metrics_dir: metrics_dir.clone(),
        agent_meta: HashMap::new(),
        cached_usage: HashMap::new(),
        health: Arc::new(Mutex::new(MetricsHealth::default())),
    };

    let records = vec![
        UsageRecord {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "agent-1".to_string(),
            agent_kind: Some("builder".to_string()),
            job_id: Some("job-1".to_string()),
            job_kind: Some("build".to_string()),
            job_step: Some("plan".to_string()),
            project: Some("myproject".to_string()),
            status: "running".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_creation_input_tokens: 100,
            cache_read_input_tokens: 200,
            total_cost_usd: Some(0.05),
            total_api_ms: Some(4500),
        },
        UsageRecord {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            agent_id: "agent-2".to_string(),
            agent_kind: None,
            job_id: None,
            job_kind: None,
            job_step: None,
            project: None,
            status: "idle".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            total_cost_usd: None,
            total_api_ms: None,
        },
    ];

    collector.write_records(&records).unwrap();

    let path = metrics_dir.join("usage.jsonl");
    let content = fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);

    // Each line should be valid JSON
    for line in &lines {
        let parsed: UsageRecord = serde_json::from_str(line).unwrap();
        assert!(!parsed.agent_id.is_empty());
    }

    // First record should have optional fields present
    let r1: UsageRecord = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(r1.agent_kind.as_deref(), Some("builder"));
    assert_eq!(r1.input_tokens, 1000);
    assert_eq!(r1.total_cost_usd, Some(0.05));
    assert_eq!(r1.total_api_ms, Some(4500));

    // Second record should skip None fields
    let r2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert!(r2.get("agent_kind").is_none());
    assert!(r2.get("total_cost_usd").is_none());
    assert!(r2.get("total_api_ms").is_none());
}

#[test]
fn rotation_shifts_files_and_writes_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let metrics_dir = dir.path().join("metrics");
    fs::create_dir_all(&metrics_dir).unwrap();

    let usage_path = metrics_dir.join("usage.jsonl");

    // Create a file that exceeds the size limit
    {
        let mut f = fs::File::create(&usage_path).unwrap();
        let dummy = "x".repeat(1024);
        for _ in 0..(MAX_METRICS_SIZE / 1024 + 1) {
            writeln!(f, "{}", dummy).unwrap();
        }
    }

    // Set up collector with one cached usage entry
    let mut cached_usage = HashMap::new();
    cached_usage.insert(
        "test-agent".to_string(),
        UsageData {
            input_tokens: 500,
            output_tokens: 250,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_cost_usd: 0.03,
            total_api_ms: 2000,
        },
    );

    let collector = UsageMetricsCollector {
        state: Arc::new(Mutex::new(MaterializedState::default())),
        agents: StubAdapter,
        metrics_dir: metrics_dir.clone(),
        agent_meta: HashMap::new(),
        cached_usage,
        health: Arc::new(Mutex::new(MetricsHealth::default())),
    };

    collector.rotate_if_needed();

    // Old file should be rotated to .1
    assert!(metrics_dir.join("usage.jsonl.1").exists());

    // New file should exist with baseline
    assert!(usage_path.exists());
    let content = fs::read_to_string(&usage_path).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1);

    let record: UsageRecord = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(record.agent_id, "test-agent");
    assert_eq!(record.input_tokens, 500);
    assert_eq!(record.output_tokens, 250);
}

#[test]
fn format_utc_now_produces_valid_timestamp() {
    let ts = format_utc_now();
    assert!(ts.len() >= 20, "timestamp too short: {ts}");
    assert!(ts.ends_with('Z'));
    assert!(ts.contains('T'));

    // Should parse as a date
    let parts: Vec<&str> = ts.split('T').collect();
    assert_eq!(parts.len(), 2);
    let date_parts: Vec<&str> = parts[0].split('-').collect();
    assert_eq!(date_parts.len(), 3);
    let year: u32 = date_parts[0].parse().unwrap();
    assert!(year >= 2025);
}
