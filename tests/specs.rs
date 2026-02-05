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
#[path = "specs/cli/run.rs"]
mod cli_run;

// project/
#[path = "specs/project/setup.rs"]
mod project_setup;

// daemon/
#[path = "specs/daemon/concurrency.rs"]
mod daemon_concurrency;
#[path = "specs/daemon/cron_job.rs"]
mod daemon_cron_job;
#[path = "specs/daemon/crons.rs"]
mod daemon_crons;
#[path = "specs/daemon/help.rs"]
mod daemon_help;
#[path = "specs/daemon/init_idempotency.rs"]
mod daemon_init_idempotency;
#[path = "specs/daemon/job_queue.rs"]
mod daemon_job_queue;
#[path = "specs/daemon/lifecycle.rs"]
mod daemon_lifecycle;
#[path = "specs/daemon/logs.rs"]
mod daemon_logs;
#[path = "specs/daemon/on_stop.rs"]
mod daemon_on_stop;
#[path = "specs/daemon/restart_queue.rs"]
mod daemon_restart_queue;
#[path = "specs/daemon/step_fallback.rs"]
mod daemon_step_fallback;
#[path = "specs/daemon/stop_hook.rs"]
mod daemon_stop_hook;
#[path = "specs/daemon/timers.rs"]
mod daemon_timers;

// job/
#[path = "specs/job/execution.rs"]
mod job_execution;
#[path = "specs/job/show.rs"]
mod job_show;
#[path = "specs/job/wait.rs"]
mod job_wait;

// agent/
#[path = "specs/agent/config.rs"]
mod agent_config;
#[path = "specs/agent/events.rs"]
mod agent_events;
#[path = "specs/agent/gates.rs"]
mod agent_gates;
#[path = "specs/agent/hooks.rs"]
mod agent_hooks;
#[path = "specs/agent/logs.rs"]
mod agent_logs;
#[path = "specs/agent/questions.rs"]
mod agent_questions;
#[path = "specs/agent/spawn.rs"]
mod agent_spawn;
#[path = "specs/agent/state_detection.rs"]
mod agent_state_detection;

// shell/
#[path = "specs/shell/invalid_syntax.rs"]
mod shell_invalid_syntax;
#[path = "specs/shell/real_world.rs"]
mod shell_real_world;
#[path = "specs/shell/valid_syntax.rs"]
mod shell_valid_syntax;
