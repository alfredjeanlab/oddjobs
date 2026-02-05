// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent definitions

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

/// Valid tmux status bar colors
pub const VALID_SESSION_COLORS: &[&str] =
    &["red", "green", "blue", "cyan", "magenta", "yellow", "white"];

/// Status bar text configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionStatusConfig {
    pub left: Option<String>,
    pub right: Option<String>,
}

/// Tmux-specific session configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TmuxSessionConfig {
    /// Status bar background color (red, green, blue, cyan, magenta, yellow, white)
    #[serde(default)]
    pub color: Option<String>,
    /// Window title string
    #[serde(default)]
    pub title: Option<String>,
    /// Status bar left/right text
    #[serde(default)]
    pub status: Option<SessionStatusConfig>,
}

/// Valid SessionStart source values for matcher filtering.
pub const VALID_PRIME_SOURCES: &[&str] = &["startup", "resume", "clear", "compact"];

/// Shell commands to run at session start for context injection.
///
/// Supports three forms:
/// - `Script`: a single shell script string (may be multi-line)
/// - `Commands`: an array of individual commands (each validated as a single shell command)
/// - `PerSource`: a map from SessionStart source to a `Script` or `Commands` definition
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum PrimeDef {
    Script(String),
    Commands(Vec<String>),
    PerSource(HashMap<String, PrimeDef>),
}

impl<'de> Deserialize<'de> for PrimeDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Script(String),
            Commands(Vec<String>),
            PerSource(HashMap<String, PrimeDef>),
        }
        match Helper::deserialize(deserializer)? {
            Helper::Script(s) => Ok(PrimeDef::Script(s)),
            Helper::Commands(v) => Ok(PrimeDef::Commands(v)),
            Helper::PerSource(map) => {
                for val in map.values() {
                    if matches!(val, PrimeDef::PerSource(_)) {
                        return Err(serde::de::Error::custom(
                            "nested per-source prime is not allowed",
                        ));
                    }
                }
                Ok(PrimeDef::PerSource(map))
            }
        }
    }
}

impl PrimeDef {
    /// Render the prime script content with variable interpolation.
    ///
    /// Only valid for `Script` and `Commands`. Panics on `PerSource` —
    /// use `render_per_source()` instead.
    #[allow(clippy::panic)]
    pub fn render(&self, vars: &HashMap<String, String>) -> String {
        match self {
            PrimeDef::Script(s) => crate::template::interpolate(s, vars),
            PrimeDef::Commands(cmds) => cmds
                .iter()
                .map(|cmd| crate::template::interpolate(cmd, vars))
                .collect::<Vec<_>>()
                .join("\n"),
            PrimeDef::PerSource(_) => {
                unreachable!("render() not valid for PerSource; use render_per_source()")
            }
        }
    }

    /// Render prime scripts per source with variable interpolation.
    ///
    /// For `PerSource`, returns a map of source name to rendered script content.
    /// For `Script`/`Commands`, returns a single-entry map with empty string key (all sources).
    pub fn render_per_source(&self, vars: &HashMap<String, String>) -> HashMap<String, String> {
        match self {
            PrimeDef::PerSource(map) => map
                .iter()
                .map(|(source, def)| (source.clone(), def.render(vars)))
                .collect(),
            other => {
                let mut m = HashMap::new();
                m.insert(String::new(), other.render(vars));
                m
            }
        }
    }
}

/// Number of times an action can fire per trigger occurrence
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Attempts {
    Finite(u32),
    Forever,
}

impl Default for Attempts {
    fn default() -> Self {
        Attempts::Finite(1) // Strong default: fire once per trigger
    }
}

impl Attempts {
    /// Check if attempts are exhausted given the current attempt count
    pub fn is_exhausted(&self, current: u32) -> bool {
        match self {
            Attempts::Finite(max) => current >= *max,
            Attempts::Forever => false,
        }
    }
}

impl<'de> Deserialize<'de> for Attempts {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum AttemptsHelper {
            Number(u32),
            String(String),
        }

        match AttemptsHelper::deserialize(deserializer)? {
            AttemptsHelper::Number(n) => {
                if n == 0 {
                    Err(D::Error::custom("attempts must be >= 1"))
                } else {
                    Ok(Attempts::Finite(n))
                }
            }
            AttemptsHelper::String(s) => {
                if s == "forever" {
                    Ok(Attempts::Forever)
                } else {
                    Err(D::Error::custom(format!(
                        "invalid attempts value '{}'; expected a positive integer or \"forever\"",
                        s
                    )))
                }
            }
        }
    }
}

/// An agent definition from the runbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    /// Agent name (set from table key, not from TOML content)
    #[serde(default)]
    pub name: String,
    /// Command to run (e.g., "claude --print")
    pub run: String,
    /// Prompt template for the agent
    #[serde(default)]
    pub prompt: Option<String>,
    /// Path to file containing prompt template
    #[serde(default)]
    pub prompt_file: Option<PathBuf>,
    /// Environment variables to set
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Working directory (relative to workspace)
    #[serde(default)]
    pub cwd: Option<String>,
    /// Shell commands to run at session start for context injection
    #[serde(default)]
    pub prime: Option<PrimeDef>,

    /// What to do when Claude is waiting for inpu
    #[serde(default)]
    pub on_idle: ActionConfig,

    /// What to do when agent process dies
    #[serde(default = "default_on_dead")]
    pub on_dead: ActionConfig,

    /// What to do when agent shows a permission/approval prompt
    #[serde(default = "default_on_prompt")]
    pub on_prompt: ActionConfig,

    /// What to do on API errors (unauthorized, credits, network)
    #[serde(default = "default_on_error")]
    pub on_error: ErrorActionConfig,

    /// What to do when agent tries to exit (Stop hook).
    /// None = context-dependent default (job: signal, standalone: escalate)
    #[serde(default)]
    pub on_stop: Option<StopActionConfig>,

    /// Maximum concurrent instances of this agent. None = unlimited.
    #[serde(default)]
    pub max_concurrency: Option<u32>,

    /// Notification messages for agent lifecycle events
    #[serde(default)]
    pub notify: crate::job::NotifyConfig,

    /// Adapter-specific session configuration (e.g., session "tmux" { ... })
    /// Keyed by provider name. Unknown providers are ignored.
    #[serde(default)]
    pub session: HashMap<String, TmuxSessionConfig>,
}

/// Action configuration - simple or with options
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ActionConfig {
    Simple(AgentAction),
    WithOptions {
        action: AgentAction,
        #[serde(default)]
        message: Option<String>,
        /// For resume: false = replace prompt (default), true = append to prompt
        #[serde(default)]
        append: bool,
        /// For check: shell command to run
        #[serde(default)]
        run: Option<String>,
        /// Number of times to attempt this action (default: 1)
        #[serde(default)]
        attempts: Attempts,
        /// Cooldown between attempts (e.g., "30s", "5m")
        #[serde(default)]
        cooldown: Option<String>,
    },
}

impl Default for ActionConfig {
    fn default() -> Self {
        ActionConfig::Simple(AgentAction::Escalate)
    }
}

impl ActionConfig {
    /// Create a simple action config with no message
    pub fn simple(action: AgentAction) -> Self {
        ActionConfig::Simple(action)
    }

    /// Create an action config with a replacement message
    pub fn with_message(action: AgentAction, message: &str) -> Self {
        ActionConfig::WithOptions {
            action,
            message: Some(message.to_string()),
            append: false,
            run: None,
            attempts: Attempts::default(),
            cooldown: None,
        }
    }

    /// Create an action config with an append message
    pub fn with_append(action: AgentAction, message: &str) -> Self {
        ActionConfig::WithOptions {
            action,
            message: Some(message.to_string()),
            append: true,
            run: None,
            attempts: Attempts::default(),
            cooldown: None,
        }
    }

    pub fn action(&self) -> &AgentAction {
        match self {
            ActionConfig::Simple(a) => a,
            ActionConfig::WithOptions { action, .. } => action,
        }
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            ActionConfig::Simple(_) => None,
            ActionConfig::WithOptions { message, .. } => message.as_deref(),
        }
    }

    pub fn append(&self) -> bool {
        match self {
            ActionConfig::Simple(_) => false,
            ActionConfig::WithOptions { append, .. } => *append,
        }
    }

    pub fn run(&self) -> Option<&str> {
        match self {
            ActionConfig::Simple(_) => None,
            ActionConfig::WithOptions { run, .. } => run.as_deref(),
        }
    }

    pub fn attempts(&self) -> Attempts {
        match self {
            ActionConfig::Simple(_) => Attempts::default(),
            ActionConfig::WithOptions { attempts, .. } => *attempts,
        }
    }

    pub fn cooldown(&self) -> Option<&str> {
        match self {
            ActionConfig::Simple(_) => None,
            ActionConfig::WithOptions { cooldown, .. } => cooldown.as_deref(),
        }
    }
}

/// Trigger contexts for agent actions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionTrigger {
    OnIdle,   // Agent waiting for input (still running)
    OnDead,   // Agent process exited
    OnError,  // API error occurred
    OnPrompt, // Agent showing a permission/approval prompt
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentAction {
    Nudge,  // Send message prompting to continue
    Done,   // Treat as success, advance job
    Fail,   // Mark job as failed
    Resume, // Re-spawn with --resume, preserving conversation history
    #[default]
    Escalate, // Notify human
    Gate,   // Run a shell command; advance if it passes, escalate if it fails
}

impl AgentAction {
    /// Returns the action name as used in TOML (lowercase).
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentAction::Nudge => "nudge",
            AgentAction::Done => "done",
            AgentAction::Fail => "fail",
            AgentAction::Resume => "resume",
            AgentAction::Escalate => "escalate",
            AgentAction::Gate => "gate",
        }
    }

    /// Returns whether this action is valid for the given trigger context.
    pub fn is_valid_for_trigger(&self, trigger: ActionTrigger) -> bool {
        match trigger {
            // on_idle: agent is still running, can't restart/resume
            ActionTrigger::OnIdle => matches!(
                self,
                AgentAction::Nudge
                    | AgentAction::Done
                    | AgentAction::Escalate
                    | AgentAction::Fail
                    | AgentAction::Gate
            ),
            // on_dead: agent exited, can't nudge a dead process
            ActionTrigger::OnDead => matches!(
                self,
                AgentAction::Done
                    | AgentAction::Resume
                    | AgentAction::Escalate
                    | AgentAction::Fail
                    | AgentAction::Gate
            ),
            // on_error: API error, resume is valid for transient errors (rate limits, etc.)
            ActionTrigger::OnError => matches!(
                self,
                AgentAction::Fail | AgentAction::Resume | AgentAction::Escalate | AgentAction::Gate
            ),
            // on_prompt: agent at a prompt, can't nudge or resume
            ActionTrigger::OnPrompt => matches!(
                self,
                AgentAction::Done | AgentAction::Escalate | AgentAction::Fail | AgentAction::Gate
            ),
        }
    }

    /// Returns a human-readable reason why this action is invalid for the trigger.
    pub fn invalid_reason(&self, trigger: ActionTrigger) -> &'static str {
        match (self, trigger) {
            (AgentAction::Resume, ActionTrigger::OnIdle) => {
                "resume is for re-spawning after exit; agent is still running"
            }
            (AgentAction::Nudge, ActionTrigger::OnDead) => {
                "nudge sends a message to a running agent; agent has exited"
            }
            (AgentAction::Nudge, ActionTrigger::OnError) => {
                "nudge sends a message to a running agent; use escalate instead"
            }
            (AgentAction::Done, ActionTrigger::OnError) => {
                "done marks success; API errors are not success states"
            }
            (AgentAction::Nudge, ActionTrigger::OnPrompt) => {
                "nudge sends a message; agent is at a prompt, not idle"
            }
            (AgentAction::Resume, ActionTrigger::OnPrompt) => {
                "resume is for re-spawning after exit; agent is still running"
            }
            _ => "action not allowed for this trigger",
        }
    }
}

/// Error action configuration - simple or per-error-type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ErrorActionConfig {
    /// Same action for all errors: on_error = "escalate"
    Simple(ActionConfig),
    /// Per-error with fallthrough: [[on_error]]
    ByType(Vec<ErrorMatch>),
}

impl Default for ErrorActionConfig {
    fn default() -> Self {
        ErrorActionConfig::Simple(ActionConfig::Simple(AgentAction::Escalate))
    }
}

impl ErrorActionConfig {
    /// Find the action config for a given error type
    pub fn action_for(&self, error_type: Option<&ErrorType>) -> ActionConfig {
        match self {
            ErrorActionConfig::Simple(config) => config.clone(),
            ErrorActionConfig::ByType(matches) => matches
                .iter()
                .find(|m| m.error_match.is_none() || m.error_match.as_ref() == error_type)
                .map(|m| ActionConfig::WithOptions {
                    action: m.action.clone(),
                    message: m.message.clone(),
                    append: m.append,
                    run: None,
                    attempts: Attempts::default(),
                    cooldown: None,
                })
                .unwrap_or_else(|| ActionConfig::Simple(AgentAction::Escalate)),
        }
    }

    /// Returns all actions configured (for validation iteration).
    pub fn all_actions(&self) -> Vec<&AgentAction> {
        match self {
            ErrorActionConfig::Simple(config) => vec![config.action()],
            ErrorActionConfig::ByType(matches) => matches.iter().map(|m| &m.action).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMatch {
    /// Error type to match (None = catch-all)
    #[serde(rename = "match")]
    pub error_match: Option<ErrorType>,
    pub action: AgentAction,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub append: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    Unauthorized,
    OutOfCredits,
    NoInternet,
    RateLimited,
}

fn default_on_dead() -> ActionConfig {
    ActionConfig::Simple(AgentAction::Escalate)
}

fn default_on_prompt() -> ActionConfig {
    ActionConfig::Simple(AgentAction::Escalate)
}

fn default_on_error() -> ErrorActionConfig {
    ErrorActionConfig::default()
}

/// What to do when the agent's Stop hook fires (agent tries to exit)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StopAction {
    /// Block exit until agent calls `oj emit agent:signal` (job default)
    Signal,
    /// Treat stop as idle — fire on_idle handler
    Idle,
    /// Block exit and escalate to human (standalone default)
    Escalate,
}

/// Configuration for the on_stop lifecycle handler
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StopActionConfig {
    Simple(StopAction),
    WithOptions { action: StopAction },
}

impl StopActionConfig {
    pub fn action(&self) -> &StopAction {
        match self {
            StopActionConfig::Simple(a) => a,
            StopActionConfig::WithOptions { action } => action,
        }
    }
}

impl Default for AgentDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            run: String::new(),
            prompt: None,
            prompt_file: None,
            env: HashMap::new(),
            cwd: None,
            prime: None,
            on_idle: ActionConfig::default(),
            on_dead: default_on_dead(),
            on_prompt: default_on_prompt(),
            on_error: default_on_error(),
            on_stop: None,
            max_concurrency: None,
            notify: Default::default(),
            session: HashMap::new(),
        }
    }
}

impl AgentDef {
    /// Build the command with interpolated variables
    pub fn build_command(&self, vars: &HashMap<String, String>) -> String {
        crate::template::interpolate(&self.run, vars)
    }

    /// Build the environment variables with interpolated values
    pub fn build_env(&self, vars: &HashMap<String, String>) -> Vec<(String, String)> {
        self.env
            .iter()
            .map(|(k, v)| (k.clone(), crate::template::interpolate(v, vars)))
            .collect()
    }

    /// Get the prompt text with variables interpolated
    ///
    /// Reads from prompt_file if specified, otherwise uses prompt field.
    /// Returns empty string if neither is set.
    pub fn get_prompt(&self, vars: &HashMap<String, String>) -> io::Result<String> {
        let template = if let Some(ref file) = self.prompt_file {
            std::fs::read_to_string(file)?
        } else if let Some(ref prompt) = self.prompt {
            prompt.clone()
        } else {
            return Ok(String::new());
        };
        Ok(crate::template::interpolate(&template, vars))
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
