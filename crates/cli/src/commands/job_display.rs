// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Display helpers for job commands.

use std::collections::HashMap;

use crate::color;

pub(crate) fn format_agent_summary(agent: &oj_wire::AgentSummary) -> String {
    let mut parts = Vec::new();
    if agent.files_read > 0 {
        parts.push(format!(
            "{} file{} read",
            agent.files_read,
            if agent.files_read == 1 { "" } else { "s" }
        ));
    }
    if agent.files_written > 0 {
        parts.push(format!(
            "{} file{} written",
            agent.files_written,
            if agent.files_written == 1 { "" } else { "s" }
        ));
    }
    if agent.commands_run > 0 {
        parts.push(format!(
            "{} command{}",
            agent.commands_run,
            if agent.commands_run == 1 { "" } else { "s" }
        ));
    }
    if let Some(ref reason) = agent.exit_reason {
        parts.push(format!("exit: {}", reason));
    }
    parts.join(", ")
}

pub(crate) fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

pub(crate) fn format_var_value(value: &str, max_len: usize) -> String {
    let escaped = value.replace('\n', "\\n");
    if escaped.chars().count() <= max_len {
        escaped
    } else {
        let truncated: String = escaped.chars().take(max_len).collect();
        format!("{}...", truncated)
    }
}

pub(crate) fn is_var_truncated(value: &str, max_len: usize) -> bool {
    let escaped = value.replace('\n', "\\n");
    escaped.chars().count() > max_len
}

/// Variable scope ordering for grouped display.
/// Returns (order_priority, scope_name) for sorting.
pub(crate) fn var_scope_order(key: &str) -> (usize, &str) {
    if let Some(dot_pos) = key.find('.') {
        let scope = &key[..dot_pos];
        let priority = match scope {
            "var" => 0,
            "local" => 1,
            "workspace" => 2,
            "invoke" => 3,
            _ => 4, // other namespaced vars
        };
        (priority, scope)
    } else {
        (5, "") // unnamespaced vars last
    }
}

/// Group and sort variables by scope for display.
pub(crate) fn group_vars_by_scope(vars: &HashMap<String, String>) -> Vec<(&String, &String)> {
    let mut sorted: Vec<_> = vars.iter().collect();
    sorted.sort_by(|(a, _), (b, _)| {
        let (order_a, scope_a) = var_scope_order(a);
        let (order_b, scope_b) = var_scope_order(b);
        order_a.cmp(&order_b).then_with(|| scope_a.cmp(scope_b)).then_with(|| a.cmp(b))
    });
    sorted
}

/// Print follow-up commands for a job.
pub(crate) fn print_job_commands(short_id: &str) {
    println!("    oj job show {short_id}");
    println!("    oj job wait {short_id}      {}", color::muted("# Wait until job ends"));
    println!("    oj job logs {short_id} -f   {}", color::muted("# Follow logs"));
    println!("    oj job peek {short_id}      {}", color::muted("# Capture agent output"));
    println!("    oj job attach {short_id}    {}", color::muted("# Attach to agent"));
}
