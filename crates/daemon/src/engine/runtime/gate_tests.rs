// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::parse_gate_error;

#[yare::parameterized(
    exit_code_no_stderr   = { "gate `make test` failed (exit 1)",                      1,    "" },
    exit_code_with_stderr = { "gate `make test` failed (exit 2): compilation error",   2,    "compilation error" },
    execution_error       = { "gate `make test` execution error: not found",           1,    "gate `make test` execution error: not found" },
    exit_code_zero        = { "gate `true` failed (exit 0)",                           0,    "" },
    high_exit_code        = { "gate `cmd` failed (exit 127)",                          127,  "" },
    multiline_stderr      = { "gate `cmd` failed (exit 1): line1\nline2\nline3",       1,    "line1\nline2\nline3" },
    no_pattern_match      = { "some random error message",                             1,    "some random error message" },
    negative_exit_code    = { "gate `cmd` failed (exit -1)",                           -1,   "" },
)]
fn parse(input: &str, expected_code: i32, expected_stderr: &str) {
    let (code, stderr) = parse_gate_error(input);
    assert_eq!(code, expected_code);
    assert_eq!(stderr, expected_stderr);
}
