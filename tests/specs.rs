//! Behavioral specifications for oj CLI.
//!
//! These tests are black-box: they invoke the CLI binary and verify
//! stdout, stderr, and exit codes. See tests/specs/CLAUDE.md for conventions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "specs/prelude.rs"]
mod prelude;

// cli/
#[path = "specs/cli/errors.rs"]
mod cli_errors;
#[path = "specs/cli/help.rs"]
mod cli_help;

// project/
#[path = "specs/project/setup.rs"]
mod project_setup;

// daemon/
#[path = "specs/daemon/help.rs"]
mod daemon_help;
#[path = "specs/daemon/lifecycle.rs"]
mod daemon_lifecycle;
#[path = "specs/daemon/logs.rs"]
mod daemon_logs;
#[path = "specs/daemon/timers.rs"]
mod daemon_timers;

// pipeline/
#[path = "specs/pipeline/execution.rs"]
mod pipeline_execution;
#[path = "specs/pipeline/show.rs"]
mod pipeline_show;
#[path = "specs/pipeline/wait.rs"]
mod pipeline_wait;

// agent/
#[path = "specs/agent/config.rs"]
mod agent_config;
#[path = "specs/agent/events.rs"]
mod agent_events;
#[path = "specs/agent/gates.rs"]
mod agent_gates;
#[path = "specs/agent/logs.rs"]
mod agent_logs;
#[path = "specs/agent/spawn.rs"]
mod agent_spawn;

// shell/
#[path = "specs/shell/invalid_syntax.rs"]
mod shell_invalid_syntax;
#[path = "specs/shell/real_world.rs"]
mod shell_real_world;
#[path = "specs/shell/valid_syntax.rs"]
mod shell_valid_syntax;
