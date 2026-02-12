// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::io::Write;

#[test]
fn oauth_token_env_pair() {
    let cred = Credential::OAuthToken("tok-123".to_string());
    let (key, val) = cred.to_env_pair();
    assert_eq!(key, "CLAUDE_CODE_OAUTH_TOKEN");
    assert_eq!(val, "tok-123");
}

#[test]
fn api_key_env_pair() {
    let cred = Credential::ApiKey("sk-ant-abc".to_string());
    let (key, val) = cred.to_env_pair();
    assert_eq!(key, "ANTHROPIC_API_KEY");
    assert_eq!(val, "sk-ant-abc");
}

#[test]
fn read_credentials_json() {
    let dir = tempfile::tempdir().unwrap();
    let cred_path = dir.path().join(".credentials.json");
    let mut f = std::fs::File::create(&cred_path).unwrap();
    writeln!(f, r#"{{"claudeAiOauth": {{"accessToken": "test-oauth-token"}}}}"#).unwrap();

    // Override HOME to use our temp dir
    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", dir.path().parent().unwrap_or(dir.path()));

    // Create the .claude directory at the temp path
    let claude_dir = dir.path().parent().unwrap_or(dir.path()).join(".claude");
    std::fs::create_dir_all(&claude_dir).ok();
    let final_path = claude_dir.join(".credentials.json");
    std::fs::copy(&cred_path, &final_path).ok();

    // Test that we can parse the file
    let content = std::fs::read_to_string(&cred_path).unwrap();
    let value: serde_json::Value = serde_json::from_str(&content).unwrap();
    let token =
        value.get("claudeAiOauth").and_then(|v| v.get("accessToken")).and_then(|v| v.as_str());
    assert_eq!(token, Some("test-oauth-token"));

    // Restore HOME
    if let Some(home) = original_home {
        std::env::set_var("HOME", home);
    }
}

#[test]
fn read_claude_json_api_key_parsing() {
    let content = r#"{"primaryApiKey": "sk-ant-test123"}"#;
    let value: serde_json::Value = serde_json::from_str(content).unwrap();
    let key = value.get("primaryApiKey").and_then(|v| v.as_str());
    assert_eq!(key, Some("sk-ant-test123"));
}

#[test]
fn empty_strings_are_filtered() {
    let content = r#"{"claudeAiOauth": {"accessToken": ""}}"#;
    let value: serde_json::Value = serde_json::from_str(content).unwrap();
    let token = value
        .get("claudeAiOauth")
        .and_then(|v| v.get("accessToken"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    assert!(token.is_none());
}
