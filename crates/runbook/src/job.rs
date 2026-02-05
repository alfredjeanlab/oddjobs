// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job definitions
//!
//! # Design Note: No Step Timeouts
//!
//! This module deliberately does not support step-level timeouts for agent steps.
//! This is a conscious design decision, not an oversight.
//!
//! ## Why No Timeout Feature?
//!
//! **This is a dynamic, monitored system.** Agents and jobs are actively watched by
//! both automated handlers (`on_idle`, `on_dead`, `on_error`) and human operators. When
//! something goes wrong, these monitoring systems detect the actual problem state—not an
//! arbitrary time threshold.
//!
//! **Agents may run for extended periods.** Agents may eventually work on complex tasks
//! taking days or weeks of actual productive work. A timeout would kill legitimate work.
//! The system must distinguish "working for a long time" from "stuck"—timeouts cannot.
//!
//! **The default must be NO timeout.** If a timeout feature existed, the only safe default
//! would be infinite (no timeout). An infinite-default timeout adds complexity and
//! misconfiguration risk without benefit.
//!
//! **Timeouts hide root causes.** If an agent is stuck, restarting it via timeout provides
//! no information about why. The monitoring system detects actual states:
//! - `on_idle`: Agent waiting for input (stuck on a prompt)
//! - `on_dead`: Agent process exited unexpectedly
//! - `on_error`: Agent hit an API or system error
//!
//! ## When Timeouts ARE Appropriate
//!
//! Timeouts make sense for bounded operations like shell commands with known
//! execution bounds — not for agent steps.
//!
//! See [`docs/01-concepts/EXECUTION.md`] for the full rationale.

use crate::command::RunDirective;
use indexmap::IndexMap;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fmt;

/// A step transition target.
///
/// Accepts either:
///   `{ step = "name" }`  — structured form (preferred)
///   `"name"`             — bare string (backward compat)
#[derive(Debug, Clone, Serialize)]
pub struct StepTransition {
    pub step: String,
}

impl StepTransition {
    pub fn step_name(&self) -> &str {
        &self.step
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StepTransitionRaw {
    Structured { step: String },
    Bare(String),
}

impl<'de> Deserialize<'de> for StepTransition {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = StepTransitionRaw::deserialize(d)?;
        Ok(match raw {
            StepTransitionRaw::Structured { step } => StepTransition { step },
            StepTransitionRaw::Bare(s) => StepTransition { step: s },
        })
    }
}

/// Notification configuration for lifecycle events
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifyConfig {
    /// Message template sent when the job/agent starts
    #[serde(default)]
    pub on_start: Option<String>,
    /// Message template sent when the job/agent completes successfully
    #[serde(default)]
    pub on_done: Option<String>,
    /// Message template sent when the job/agent fails
    #[serde(default)]
    pub on_fail: Option<String>,
}

impl NotifyConfig {
    /// Render a message template with variable interpolation.
    pub fn render(template: &str, vars: &std::collections::HashMap<String, String>) -> String {
        crate::template::interpolate(template, vars)
    }
}

/// Workspace configuration for job execution.
///
/// Supports two forms:
///   `workspace = "folder"`                    — plain directory
///   `workspace { git = "worktree" }`          — git worktree (engine-managed)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkspaceConfig {
    /// Short form: `workspace = "folder"`
    Simple(WorkspaceType),
    /// Block form: `workspace { git = "worktree" }`
    Block(WorkspaceBlock),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceType {
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBlock {
    pub git: GitWorkspaceMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub from_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitWorkspaceMode {
    Worktree,
}

impl WorkspaceConfig {
    pub fn is_git_worktree(&self) -> bool {
        matches!(
            self,
            WorkspaceConfig::Block(WorkspaceBlock {
                git: GitWorkspaceMode::Worktree,
                ..
            })
        )
    }
}

/// A step within a job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDef {
    /// Step name (injected from map key in HCL format)
    #[serde(default)]
    pub name: String,
    /// What to run: shell command or agent
    pub run: RunDirective,
    /// Next step on success
    #[serde(default)]
    pub on_done: Option<StepTransition>,
    /// Step to go to on failure
    #[serde(default)]
    pub on_fail: Option<StepTransition>,
    /// Step to route to when the job is cancelled during this step
    #[serde(default)]
    pub on_cancel: Option<StepTransition>,
}

impl StepDef {
    /// Check if this step runs a shell command
    pub fn is_shell(&self) -> bool {
        self.run.is_shell()
    }

    /// Check if this step invokes an agent
    pub fn is_agent(&self) -> bool {
        self.run.is_agent()
    }

    /// Get the agent name if this step invokes an agent
    pub fn agent_name(&self) -> Option<&str> {
        self.run.agent_name()
    }

    /// Get the shell command if this step runs a shell command
    pub fn shell_command(&self) -> Option<&str> {
        self.run.shell_command()
    }
}

/// A job definition from the runbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDef {
    /// Job kind (injected from HCL block label, e.g. `job "build"` → kind = "build")
    #[serde(default)]
    pub kind: String,
    /// Optional name template for human-readable job names.
    /// Supports `${var.*}` interpolation. The result is slugified and
    /// suffixed with a nonce derived from the job UUID.
    #[serde(default)]
    pub name: Option<String>,
    /// Required variables
    #[serde(default, alias = "input")]
    pub vars: Vec<String>,
    /// Default values for input
    #[serde(default)]
    pub defaults: HashMap<String, String>,
    /// Base directory or repo path for execution (supports template interpolation)
    #[serde(default)]
    pub cwd: Option<String>,
    /// Workspace configuration: "folder" (plain dir) or `{ git = "worktree" }` (engine-managed)
    #[serde(default)]
    pub workspace: Option<WorkspaceConfig>,
    /// Step to route to when the job completes (no step-level on_done)
    #[serde(default)]
    pub on_done: Option<StepTransition>,
    /// Step to route to when a step fails (no step-level on_fail)
    #[serde(default)]
    pub on_fail: Option<StepTransition>,
    /// Step to route to when the job is cancelled (no step-level on_cancel)
    #[serde(default)]
    pub on_cancel: Option<StepTransition>,
    /// Local variables computed at job creation time.
    /// Values are template strings evaluated once, available as ${local.*}.
    #[serde(default)]
    pub locals: HashMap<String, String>,
    /// Notification messages for job lifecycle events
    #[serde(default)]
    pub notify: NotifyConfig,
    /// Ordered steps
    #[serde(default, alias = "step", deserialize_with = "deserialize_steps")]
    pub steps: Vec<StepDef>,
}

/// Deserialize steps from either a sequence (TOML) or a map (HCL labeled blocks).
///
/// - TOML `[[job.X.step]]` produces a `Vec<StepDef>`
/// - HCL `step "name" { }` produces an `IndexMap<String, StepDef>` (preserves insertion order)
fn deserialize_steps<'de, D>(deserializer: D) -> Result<Vec<StepDef>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StepsVisitor;

    impl<'de> Visitor<'de> for StepsVisitor {
        type Value = Vec<StepDef>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a sequence of steps or a map of labeled step blocks")
        }

        fn visit_seq<S>(self, seq: S) -> Result<Vec<StepDef>, S::Error>
        where
            S: SeqAccess<'de>,
        {
            Vec::deserialize(de::value::SeqAccessDeserializer::new(seq))
        }

        fn visit_map<M>(self, map: M) -> Result<Vec<StepDef>, M::Error>
        where
            M: MapAccess<'de>,
        {
            let index_map: IndexMap<String, StepDef> =
                IndexMap::deserialize(de::value::MapAccessDeserializer::new(map))?;
            Ok(index_map
                .into_iter()
                .map(|(key, mut step)| {
                    if step.name.is_empty() {
                        step.name = key;
                    }
                    step
                })
                .collect())
        }
    }

    deserializer.deserialize_any(StepsVisitor)
}

impl JobDef {
    /// Get a step by name
    pub fn get_step(&self, name: &str) -> Option<&StepDef> {
        self.steps.iter().find(|p| p.name == name)
    }

    /// Get the first step
    pub fn first_step(&self) -> Option<&StepDef> {
        self.steps.first()
    }
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod tests;
