// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn agent_state_display() {
    assert_eq!(AgentState::Working.to_string(), "working");
    assert_eq!(AgentState::WaitingForInput.to_string(), "waiting for input");
    assert_eq!(AgentState::Failed(AgentError::Unauthorized).to_string(), "failed: unauthorized");
    assert_eq!(AgentState::Exited { exit_code: Some(0) }.to_string(), "exited with code 0");
    assert_eq!(AgentState::Exited { exit_code: None }.to_string(), "exited");
    assert_eq!(AgentState::SessionGone.to_string(), "session gone");
}

#[test]
fn agent_failure_display() {
    assert_eq!(AgentError::Unauthorized.to_string(), "unauthorized");
    assert_eq!(AgentError::OutOfCredits.to_string(), "out of credits");
    assert_eq!(AgentError::NoInternet.to_string(), "no internet connection");
    assert_eq!(AgentError::RateLimited.to_string(), "rate limited");
    assert_eq!(AgentError::Other("test".into()).to_string(), "test");
}
