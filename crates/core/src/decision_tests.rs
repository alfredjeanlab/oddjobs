// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn decision_source_serde_roundtrip() {
    let sources = vec![
        DecisionSource::Question,
        DecisionSource::Approval,
        DecisionSource::Gate,
        DecisionSource::Error,
        DecisionSource::Idle,
    ];
    for source in sources {
        let json = serde_json::to_string(&source).unwrap();
        let parsed: DecisionSource = serde_json::from_str(&json).unwrap();
        assert_eq!(source, parsed);
    }
}

#[test]
fn decision_option_serde_roundtrip() {
    let opt = DecisionOption {
        label: "Retry".to_string(),
        description: Some("Try the step again".to_string()),
        recommended: true,
    };
    let json = serde_json::to_string(&opt).unwrap();
    let parsed: DecisionOption = serde_json::from_str(&json).unwrap();
    assert_eq!(opt, parsed);
}

#[test]
fn decision_option_minimal_serde() {
    let opt = DecisionOption {
        label: "Skip".to_string(),
        description: None,
        recommended: false,
    };
    let json = serde_json::to_string(&opt).unwrap();
    // description should be omitted when None
    assert!(!json.contains("description"));
    let parsed: DecisionOption = serde_json::from_str(&json).unwrap();
    assert_eq!(opt, parsed);
}

#[test]
fn decision_serde_roundtrip() {
    let decision = Decision {
        id: DecisionId::new("dec-123"),
        job_id: "pipe-1".to_string(),
        agent_id: Some("agent-1".to_string()),
        owner: None,
        source: DecisionSource::Gate,
        context: "Gate failed with exit code 1".to_string(),
        options: vec![
            DecisionOption {
                label: "Retry".to_string(),
                description: None,
                recommended: true,
            },
            DecisionOption {
                label: "Skip".to_string(),
                description: Some("Skip this step".to_string()),
                recommended: false,
            },
        ],
        chosen: None,
        message: None,
        created_at_ms: 1_000_000,
        resolved_at_ms: None,
        namespace: "myproject".to_string(),
    };
    let json = serde_json::to_string(&decision).unwrap();
    let parsed: Decision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.id, DecisionId::new("dec-123"));
    assert_eq!(parsed.job_id, "pipe-1");
    assert_eq!(parsed.source, DecisionSource::Gate);
    assert_eq!(parsed.options.len(), 2);
    assert!(!parsed.is_resolved());
}

#[test]
fn decision_is_resolved() {
    let mut decision = Decision {
        id: DecisionId::new("dec-1"),
        job_id: "pipe-1".to_string(),
        agent_id: None,
        owner: None,
        source: DecisionSource::Question,
        context: "What should we do?".to_string(),
        options: vec![],
        chosen: None,
        message: None,
        created_at_ms: 1_000_000,
        resolved_at_ms: None,
        namespace: String::new(),
    };
    assert!(!decision.is_resolved());

    decision.resolved_at_ms = Some(2_000_000);
    assert!(decision.is_resolved());
}

#[test]
fn decision_id_display() {
    let id = DecisionId::new("abc-123");
    assert_eq!(format!("{}", id), "abc-123");
    assert_eq!(id.as_str(), "abc-123");
}

#[test]
fn decision_id_from_conversions() {
    let id1: DecisionId = "test".into();
    let id2: DecisionId = String::from("test").into();
    assert_eq!(id1, id2);
    assert_eq!(id1, *"test");
}
