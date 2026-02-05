// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for standalone agent run lifecycle handling.
//!
//! Tests cover:
//! - Attempt tracking and exhaustion with cooldowns
//! - Gate command execution (success/failure/error)
//! - Fail action effects
//! - Signal handling and auto-resume
//! - Nudge timestamp tracking
//! - Agent lifecycle (registration, liveness, idle grace)

mod actions;
mod attempts;
mod lifecycle;
mod signals;

use super::*;
use oj_adapters::SessionCall;
use oj_core::{AgentRunId, AgentRunStatus, AgentSignalKind, OwnerId, TimerId};

// =============================================================================
// Runbook definitions for standalone agent tests
// =============================================================================

/// Runbook with standalone agent, on_idle with attempts
const RUNBOOK_AGENT_IDLE_ATTEMPTS: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = { action = "nudge", attempts = 2, message = "Keep going" }
"#;

/// Runbook with standalone agent, on_idle with attempts and cooldown
const RUNBOOK_AGENT_IDLE_COOLDOWN: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = { action = "nudge", attempts = 3, cooldown = "30s", message = "Continue" }
"#;

/// Runbook with standalone agent, on_dead = fail
const RUNBOOK_AGENT_DEAD_FAIL: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_dead = "fail"
"#;

/// Runbook with standalone agent, on_dead = gate (passing)
const RUNBOOK_AGENT_GATE_PASS: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_dead = { action = "gate", run = "true" }
on_idle = "done"
"#;

/// Runbook with standalone agent, on_dead = gate (failing)
const RUNBOOK_AGENT_GATE_FAIL: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_dead = { action = "gate", run = "false" }
"#;

/// Runbook with standalone agent for recovery testing
const RUNBOOK_AGENT_RECOVERY: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = "escalate"
on_dead = "escalate"
"#;

/// Runbook with on_error = fail
const RUNBOOK_AGENT_ERROR_FAIL: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_error = "fail"
on_idle = "done"
"#;
