//! Daemon logs specs
//!
//! Verify daemon logs command behavior.

use crate::prelude::*;

#[test]
fn daemon_logs_shows_output() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["daemon", "logs", "-n", "10"]).passes().stdout_has("ojd: starting");
}

#[test]
fn daemon_logs_shows_startup_info() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    // "Daemon ready" is written via non-blocking tracing appender, so it may
    // not be flushed to disk by the time `daemon start` returns (which only
    // waits for the socket). Poll until the log line appears.
    let ready = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["daemon", "logs"]).passes().stdout().contains("Daemon ready")
    });
    assert!(ready, "daemon log should contain 'Daemon ready'");
}
