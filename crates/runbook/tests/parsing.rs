// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Integration tests for runbook parsing across TOML, JSON, and HCL formats.

#![allow(clippy::unwrap_used, clippy::panic)]

use oj_runbook::{parse_runbook, parse_runbook_with_format, Format, ParseError, Runbook};

#[path = "parsing/action_trigger.rs"]
mod action_trigger;
#[path = "parsing/agents.rs"]
mod agents;
#[path = "parsing/cron.rs"]
mod cron;
#[path = "parsing/epic.rs"]
mod epic;
#[path = "parsing/errors.rs"]
mod errors;
#[path = "parsing/formats.rs"]
mod formats;
#[path = "parsing/prime.rs"]
mod prime;
#[path = "parsing/queues.rs"]
mod queues;
#[path = "parsing/references.rs"]
mod references;
#[path = "parsing/template_refs.rs"]
mod template_refs;

// ---------------------------------------------------------------------------
// Shared test helpers
// ---------------------------------------------------------------------------

pub(crate) fn parse_hcl(input: &str) -> Runbook {
    parse_runbook_with_format(input, Format::Hcl).unwrap()
}

pub(crate) fn parse_json(input: &str) -> Runbook {
    parse_runbook_with_format(input, Format::Json).unwrap()
}

pub(crate) fn assert_err_contains(err: &ParseError, fragments: &[&str]) {
    let msg = err.to_string();
    for frag in fragments {
        assert!(msg.contains(frag), "error should contain '{frag}': {msg}");
    }
}

pub(crate) fn assert_toml_err(input: &str, fragments: &[&str]) {
    assert_err_contains(&parse_runbook(input).unwrap_err(), fragments);
}

pub(crate) fn assert_hcl_err(input: &str, fragments: &[&str]) {
    assert_err_contains(
        &parse_runbook_with_format(input, Format::Hcl).unwrap_err(),
        fragments,
    );
}
