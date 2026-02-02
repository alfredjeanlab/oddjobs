//! Daemon help and version specs
//!
//! Verify ojd --help, --version, and related flags work without
//! acquiring the daemon lock (no startup attempt).

use crate::prelude::*;
use std::process::Command;

fn ojd() -> Command {
    Command::new(ojd_binary())
}

#[test]
fn ojd_version_shows_version_and_hash() {
    let output = ojd().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("ojd 0.1.0+"),
        "expected version with commit hash, got: {stdout}"
    );
}

#[test]
fn ojd_short_version_shows_version_and_hash() {
    let output = ojd().arg("-v").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("ojd 0.1.0+"),
        "expected version with commit hash, got: {stdout}"
    );
}

#[test]
fn ojd_capital_v_shows_version() {
    let output = ojd().arg("-V").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("ojd 0.1.0+"),
        "expected version with commit hash, got: {stdout}"
    );
}

#[test]
fn ojd_help_shows_usage() {
    let output = ojd().arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("USAGE:"), "expected USAGE section, got: {stdout}");
    assert!(stdout.contains("--help"), "expected --help in output");
    assert!(stdout.contains("--version"), "expected --version in output");
}

#[test]
fn ojd_short_help_shows_usage() {
    let output = ojd().arg("-h").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("USAGE:"), "expected USAGE section, got: {stdout}");
}

#[test]
fn ojd_help_subcommand_shows_usage() {
    let output = ojd().arg("help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("USAGE:"), "expected USAGE section, got: {stdout}");
}

#[test]
fn ojd_unknown_arg_fails() {
    let output = ojd().arg("--bogus").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unexpected argument"),
        "expected error message, got: {stderr}"
    );
}
