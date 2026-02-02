// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj run <command> [args]` - Run a command from the runbook

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use clap::Args;

use crate::client::DaemonClient;

#[derive(Args)]
pub struct RunArgs {
    /// Command to run (e.g., "build")
    pub command: String,

    /// Positional arguments for the command
    #[arg(trailing_var_arg = true)]
    pub args: Vec<String>,

    /// Named arguments (key=value)
    #[arg(short = 'a', long = "arg", value_parser = parse_key_val)]
    pub named_args: Vec<(String, String)>,

    /// Runbook file to load (e.g., "build.toml")
    #[arg(long = "runbook")]
    pub runbook: Option<String>,
}

fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid key=value: no `=` found in `{s}`"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

/// Extract `-a key=value` / `--arg key=value` entries from raw args.
///
/// When `trailing_var_arg` is active, clap captures these as positional strings
/// if they appear after the first positional argument. This function pulls them
/// out so they can be handled as named arguments.
fn extract_named_args(raw: &[String]) -> (Vec<String>, Vec<(String, String)>) {
    let mut remaining = Vec::new();
    let mut named = Vec::new();
    let mut i = 0;

    while i < raw.len() {
        if (raw[i] == "-a" || raw[i] == "--arg") && i + 1 < raw.len() {
            if let Ok(kv) = parse_key_val(&raw[i + 1]) {
                named.push(kv);
                i += 2;
                continue;
            }
        }
        remaining.push(raw[i].clone());
        i += 1;
    }

    (remaining, named)
}

/// Quick validation that a command exists in the runbook.
/// This is a lightweight check before sending to daemon.
/// Returns the loaded runbook on success for arg validation.
pub fn prevalidate_command(
    project_root: &Path,
    command: &str,
    runbook_file: Option<&str>,
) -> Result<oj_runbook::Runbook> {
    let runbook = crate::load_runbook(project_root, command, runbook_file)?;
    if runbook.get_command(command).is_none() {
        bail!("unknown command: {}", command);
    }
    Ok(runbook)
}

pub async fn handle(
    args: RunArgs,
    client: &DaemonClient,
    project_root: &Path,
    invoke_dir: &Path,
    namespace: &str,
) -> Result<()> {
    // Prevalidate locally for fast feedback
    let runbook = prevalidate_command(project_root, &args.command, args.runbook.as_deref())?;
    // Safe: prevalidate_command already verified the command exists
    let Some(cmd_def) = runbook.get_command(&args.command) else {
        bail!("command not found: {}", args.command);
    };

    // Pre-extract any -a/--arg entries from trailing args (clap's trailing_var_arg
    // captures them as positional when they appear after the first positional arg)
    let (remaining_raw, extra_named) = extract_named_args(&args.args);

    // Split remaining raw args into positional and named based on command's ArgSpec
    let (positional, mut named) = cmd_def.args.split_raw_args(&remaining_raw);

    // Merge: extracted -a args, then explicit clap -a args (later takes precedence)
    for (k, v) in extra_named {
        named.insert(k, v);
    }
    for (k, v) in args.named_args {
        named.insert(k, v);
    }

    cmd_def.validate_args(&positional, &named)?;

    // Send to daemon
    let (pipeline_id, pipeline_name) = client
        .run_command(
            project_root,
            invoke_dir,
            namespace,
            &args.command,
            &positional,
            &named,
        )
        .await?;

    let short_id = &pipeline_id[..12.min(pipeline_id.len())];
    println!("Command {} invoked.", args.command);
    println!("Waiting for pipeline to start... (Ctrl+C to skip)");
    println!();

    // Poll for pipeline start
    let wait_ms = std::env::var("OJ_RUN_WAIT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(10_000);
    let poll_interval = Duration::from_millis(500);
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    let mut started = false;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {
                if Instant::now() >= deadline {
                    break;
                }
                if let Ok(Some(p)) = client.get_pipeline(&pipeline_id).await {
                    if p.step_status != "Pending" {
                        started = true;
                        break;
                    }
                }
            }
        }
    }

    if started {
        println!("Started {} (pipeline: {})", pipeline_name, short_id);
    } else {
        println!("Still waiting for the pipeline to start, check:");
    }
    super::pipeline::print_pipeline_commands(short_id);

    Ok(())
}
