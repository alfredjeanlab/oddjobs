// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::session::{FakeSessionAdapter, SessionCall};
use std::io::Write;
use tempfile::TempDir;

mod incremental_parser;
mod lifecycle;
mod parse_state;
mod watch_loop;

/// Append a JSONL line to a file (simulates real session log appends).
fn append_line(path: &Path, content: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(f, "{}", content).unwrap();
}
