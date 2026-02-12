//! External queue dispatch specs
//!
//! Verify that external queues correctly handle items with various ID types,
//! including numeric IDs from tools like GitHub issue trackers.

use crate::prelude::*;

/// Runbook with an external queue that reads items from a JSON file.
/// The list command returns items once (deletes the file after reading).
/// The take command is a no-op (always succeeds).
/// Poll interval is fast for testing.
const NUMERIC_ID_RUNBOOK: &str = r#"
[queue.tasks]
list = "sh .oj/poll.sh"
take = "echo taken"
poll = "500ms"

[worker.runner]
run = { job = "process" }
source = { queue = "tasks" }
concurrency = 3

[job.process]

[[job.process.step]]
name = "work"
run = "echo done"
"#;

/// External queue items with numeric ID fields (e.g. {"id": 123}) should
/// all be dispatched independently. Before the fix, `as_str()` returned
/// None for numeric JSON values, collapsing every item to "unknown" and
/// causing the inflight guard to skip all but the first item.
///
/// Would have caught commit e38ef47 where numeric IDs were treated as
/// "unknown", deduplicating distinct items.
#[test]
fn external_queue_numeric_ids_dispatched_independently() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", NUMERIC_ID_RUNBOOK);

    // Items with numeric IDs (like GitHub issue numbers)
    temp.file(
        ".oj/queue-items.json",
        r#"[{"id": 123, "title": "first"}, {"id": 456, "title": "second"}]"#,
    );

    // Poll script: returns items on first call, empty array on subsequent calls.
    // Deletes the source file after reading to prevent re-dispatch after completion.
    temp.file(
        ".oj/poll.sh",
        "#!/bin/sh\n\
         F=\".oj/queue-items.json\"\n\
         if [ -f \"$F\" ]; then\n\
         \tcat \"$F\"\n\
         \trm \"$F\"\n\
         else\n\
         \techo '[]'\n\
         fi\n",
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Both items should be dispatched independently (not collapsed to "unknown").
    // If numeric IDs were broken, only 1 job would be created because the
    // inflight guard would skip the second item (same "unknown" key).
    let both_dispatched = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let jobs = temp.oj().args(&["job", "list"]).passes().stdout();
        jobs.matches("completed").count() >= 2
    });

    if !both_dispatched {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        eprintln!(
            "=== JOBS ===\n{}\n=== END JOBS ===",
            temp.oj().args(&["job", "list"]).passes().stdout()
        );
    }
    assert!(
        both_dispatched,
        "both items with numeric IDs should be dispatched as independent jobs"
    );
}
