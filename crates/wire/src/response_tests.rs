// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Backward compatibility tests for Response deserialization.

use super::*;

#[test]
fn status_orphan_count_defaults_to_zero() {
    let json = r#"{"type":"Status","uptime_secs":60,"jobs_active":1,"sessions_active":0}"#;
    let decoded: Response = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Response::Status { orphan_count, .. } => assert_eq!(orphan_count, 0),
        _ => panic!("Expected Status response"),
    }
}
