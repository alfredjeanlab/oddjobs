// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{CrewId, JobId, OwnerId};

#[test]
fn serializes_as_string() {
    let job = OwnerId::Job(JobId::from_string("job-abc123"));
    assert_eq!(serde_json::to_string(&job).unwrap(), r#""job-abc123""#);

    let crew = OwnerId::Crew(CrewId::from_string("crw-xyz789"));
    assert_eq!(serde_json::to_string(&crew).unwrap(), r#""crw-xyz789""#);
}

#[test]
fn deserializes_from_string() {
    let owner: OwnerId = serde_json::from_str(r#""job-test123""#).unwrap();
    assert!(matches!(owner, OwnerId::Job(_)));

    let owner: OwnerId = serde_json::from_str(r#""crw-run456""#).unwrap();
    assert!(matches!(owner, OwnerId::Crew(_)));
}

#[test]
fn roundtrip() {
    let original = OwnerId::Job(JobId::from_string("job-test123"));
    let json = serde_json::to_string(&original).unwrap();
    let decoded: OwnerId = serde_json::from_str(&json).unwrap();
    assert_eq!(original, decoded);
}
