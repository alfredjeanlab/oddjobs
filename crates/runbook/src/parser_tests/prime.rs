// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{
    parse_runbook, parse_runbook_with_format, Format, ParseError, PrimeDef, VALID_PRIME_SOURCES,
};

#[test]
fn parse_agent_with_prime_string() {
    let toml = r#"
[agent.worker]
run = "claude"
prime = "echo hello\ngit status"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(matches!(agent.prime, Some(PrimeDef::Script(_))));
}

#[test]
fn parse_agent_with_prime_array() {
    let toml = r#"
[agent.worker]
run = "claude"
prime = ["echo hello", "git status --short"]
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(matches!(agent.prime, Some(PrimeDef::Commands(_))));
}

#[test]
fn error_prime_array_invalid_shell() {
    let toml = r#"
[agent.worker]
run = "claude"
prime = ["echo hello", "echo 'unterminated"]
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("agent.worker.prime[1]"),
        "error should mention prime location: {}",
        msg
    );
}

#[test]
fn parse_agent_with_prime_string_multiline() {
    // String form allows multi-line scripts (no per-command validation)
    let toml = r#"
[agent.worker]
run = "claude"
prime = """
echo '## Git Status'
git status --short | head -10
if [ -f PLAN.md ]; then
  cat PLAN.md
fi
"""
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(matches!(agent.prime, Some(PrimeDef::Script(_))));
}

#[test]
fn parse_hcl_agent_with_prime_array() {
    let hcl = r#"
agent "worker" {
  run   = "claude"
  prime = ["echo hello", "git status --short"]
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(matches!(agent.prime, Some(PrimeDef::Commands(_))));
}

#[test]
fn parse_agent_without_prime() {
    let toml = r#"
[agent.worker]
run = "claude"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(agent.prime.is_none());
}

#[test]
fn parse_hcl_agent_with_per_source_prime() {
    let hcl = r#"
agent "worker" {
  run = "claude"
  prime {
    startup = ["echo startup", "git status --short"]
    resume  = ["echo resume"]
  }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = &runbook.agents["worker"];
    match &agent.prime {
        Some(PrimeDef::PerSource(map)) => {
            assert_eq!(map.len(), 2);
            assert!(map.contains_key("startup"));
            assert!(map.contains_key("resume"));
            assert!(matches!(map["startup"], PrimeDef::Commands(_)));
            assert!(matches!(map["resume"], PrimeDef::Commands(_)));
        }
        other => panic!("expected PerSource, got {:?}", other),
    }
}

#[test]
fn parse_hcl_agent_with_per_source_prime_string_values() {
    let hcl = r#"
agent "worker" {
  run = "claude"
  prime {
    startup = "echo startup"
    compact = "echo compact"
  }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = &runbook.agents["worker"];
    match &agent.prime {
        Some(PrimeDef::PerSource(map)) => {
            assert_eq!(map.len(), 2);
            assert!(matches!(map["startup"], PrimeDef::Script(_)));
            assert!(matches!(map["compact"], PrimeDef::Script(_)));
        }
        other => panic!("expected PerSource, got {:?}", other),
    }
}

#[test]
fn error_per_source_prime_invalid_source() {
    let hcl = r#"
agent "worker" {
  run = "claude"
  prime {
    startup  = ["echo startup"]
    bogus    = ["echo invalid"]
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unknown prime source 'bogus'"),
        "error should mention unknown source: {}",
        msg
    );
    // Should list valid sources
    for source in VALID_PRIME_SOURCES {
        assert!(
            msg.contains(source),
            "error should list valid source '{}': {}",
            source,
            msg
        );
    }
}

#[test]
fn error_per_source_prime_invalid_shell() {
    let hcl = r#"
agent "worker" {
  run = "claude"
  prime {
    startup = ["echo 'unterminated"]
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(
        matches!(err, ParseError::ShellError { .. }),
        "expected ShellError, got: {:?}",
        err
    );
    let msg = err.to_string();
    assert!(
        msg.contains("agent.worker.prime.startup[0]"),
        "error should mention per-source location: {}",
        msg
    );
}

#[test]
fn parse_toml_agent_with_per_source_prime() {
    let toml = r#"
[agent.worker]
run = "claude"

[agent.worker.prime]
startup = ["echo startup", "git status"]
resume = "echo resume"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = &runbook.agents["worker"];
    match &agent.prime {
        Some(PrimeDef::PerSource(map)) => {
            assert_eq!(map.len(), 2);
            assert!(matches!(map["startup"], PrimeDef::Commands(_)));
            assert!(matches!(map["resume"], PrimeDef::Script(_)));
        }
        other => panic!("expected PerSource, got {:?}", other),
    }
}
