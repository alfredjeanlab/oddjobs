// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Credential resolution for containerized agents.
//!
//! Containerized agents run in isolated environments without access to the
//! host's keychain or config files. The daemon resolves credentials from the
//! host at spawn time and injects them as environment variables.
//!
//! Resolution follows the same fallback chain as coop/Claude Code:
//!
//! ```text
//! Flow A — OAuth token (preferred):
//!   1. CLAUDE_CODE_OAUTH_TOKEN env var
//!   2. macOS Keychain ("Claude Code-credentials")
//!   3. ~/.claude/.credentials.json → claudeAiOauth.accessToken
//!
//! Flow B — API key (fallback):
//!   4. ANTHROPIC_API_KEY env var
//!   5. ~/.claude/.claude.json → primaryApiKey
//! ```

use std::path::PathBuf;

/// A resolved credential for injecting into a container.
#[derive(Debug, Clone)]
pub enum Credential {
    /// OAuth token — injected as `CLAUDE_CODE_OAUTH_TOKEN`.
    OAuthToken(String),
    /// API key — injected as `ANTHROPIC_API_KEY`.
    ApiKey(String),
}

impl Credential {
    /// Returns the environment variable name and value for this credential.
    pub fn to_env_pair(&self) -> (&str, &str) {
        match self {
            Credential::OAuthToken(token) => ("CLAUDE_CODE_OAUTH_TOKEN", token),
            Credential::ApiKey(key) => ("ANTHROPIC_API_KEY", key),
        }
    }
}

/// Resolve a credential from the host environment.
///
/// Walks the fallback chain and returns the first valid credential found.
/// Returns `None` if no credential is available (agent will likely fail to auth).
pub fn resolve() -> Option<Credential> {
    // Flow A: OAuth token
    if let Some(cred) = resolve_oauth() {
        return Some(cred);
    }

    // Flow B: API key
    resolve_api_key()
}

/// Attempt OAuth token resolution (env → keychain → credentials file).
fn resolve_oauth() -> Option<Credential> {
    // 1. Environment variable
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        if !token.is_empty() {
            return Some(Credential::OAuthToken(token));
        }
    }

    // 2. macOS Keychain
    #[cfg(target_os = "macos")]
    if let Some(token) = read_keychain_token() {
        return Some(Credential::OAuthToken(token));
    }

    // 3. credentials.json file
    if let Some(token) = read_credentials_file() {
        return Some(Credential::OAuthToken(token));
    }

    None
}

/// Attempt API key resolution (env → claude.json file).
fn resolve_api_key() -> Option<Credential> {
    // 4. Environment variable
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return Some(Credential::ApiKey(key));
        }
    }

    // 5. claude.json file
    if let Some(key) = read_claude_json_api_key() {
        return Some(Credential::ApiKey(key));
    }

    None
}

/// Read OAuth token from macOS Keychain.
#[cfg(target_os = "macos")]
fn read_keychain_token() -> Option<String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let raw = String::from_utf8(output.stdout).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    // The keychain stores a JSON blob; extract the access token
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    value
        .get("claudeAiOauth")
        .and_then(|v| v.get("accessToken"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Read OAuth token from `~/.claude/.credentials.json`.
fn read_credentials_file() -> Option<String> {
    let path = claude_dir()?.join(".credentials.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("claudeAiOauth")
        .and_then(|v| v.get("accessToken"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Read API key from `~/.claude/.claude.json`.
fn read_claude_json_api_key() -> Option<String> {
    let path = claude_dir()?.join(".claude.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("primaryApiKey")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Resolve `~/.claude` directory.
fn claude_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|home| PathBuf::from(home).join(".claude"))
}

#[cfg(test)]
#[path = "credential_tests.rs"]
mod tests;
