// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use oj_runbook::{parse_runbook, ParseError, PrimeDef, VALID_PRIME_SOURCES};

#[test]
fn prime_string() {
    let agent = &parse_runbook("[agent.w]\nrun = \"claude\"\nprime = \"echo hello\\ngit status\"")
        .unwrap()
        .agents["w"];
    assert!(matches!(agent.prime, Some(PrimeDef::Script(_))));
}

#[test]
fn prime_array() {
    let agent = &parse_runbook(
        "[agent.w]\nrun = \"claude\"\nprime = [\"echo hello\", \"git status --short\"]",
    )
    .unwrap()
    .agents["w"];
    assert!(matches!(agent.prime, Some(PrimeDef::Commands(_))));
}

#[test]
fn error_prime_array_invalid_shell() {
    let toml = "[agent.w]\nrun = \"claude\"\nprime = [\"echo hello\", \"echo 'unterminated\"]";
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    super::assert_err_contains(&err, &["agent.w.prime[1]"]);
}

#[test]
fn prime_string_multiline() {
    let toml = r#"
[agent.w]
run = "claude"
prime = """
echo '## Git Status'
git status --short | head -10
if [ -f PLAN.md ]; then
  cat PLAN.md
fi
"""
"#;
    assert!(matches!(
        parse_runbook(toml).unwrap().agents["w"].prime,
        Some(PrimeDef::Script(_))
    ));
}

#[test]
fn hcl_prime_array() {
    let agent = &super::parse_hcl(
        "agent \"w\" {\n  run = \"claude\"\n  prime = [\"echo hello\", \"git status --short\"]\n}",
    )
    .agents["w"];
    assert!(matches!(agent.prime, Some(PrimeDef::Commands(_))));
}

#[test]
fn prime_absent() {
    assert!(
        parse_runbook("[agent.w]\nrun = \"claude\"").unwrap().agents["w"]
            .prime
            .is_none()
    );
}

#[test]
fn hcl_per_source_prime() {
    let hcl = r#"
agent "w" {
  run = "claude"
  prime {
    startup = ["echo startup", "git status --short"]
    resume  = ["echo resume"]
  }
}
"#;
    match &super::parse_hcl(hcl).agents["w"].prime {
        Some(PrimeDef::PerSource(map)) => {
            assert_eq!(map.len(), 2);
            assert!(matches!(map["startup"], PrimeDef::Commands(_)));
            assert!(matches!(map["resume"], PrimeDef::Commands(_)));
        }
        other => panic!("expected PerSource, got {:?}", other),
    }
}

#[test]
fn hcl_per_source_prime_string_values() {
    let hcl = r#"
agent "w" {
  run = "claude"
  prime {
    startup = "echo startup"
    compact = "echo compact"
  }
}
"#;
    match &super::parse_hcl(hcl).agents["w"].prime {
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
agent "w" {
  run = "claude"
  prime {
    startup = ["echo startup"]
    bogus   = ["echo invalid"]
  }
}
"#;
    let err = oj_runbook::parse_runbook_with_format(hcl, oj_runbook::Format::Hcl).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("unknown prime source 'bogus'"), "got: {msg}");
    for source in VALID_PRIME_SOURCES {
        assert!(
            msg.contains(source),
            "should list valid source '{source}': {msg}"
        );
    }
}

#[test]
fn error_per_source_prime_invalid_shell() {
    let hcl = r#"
agent "w" {
  run = "claude"
  prime { startup = ["echo 'unterminated"] }
}
"#;
    let err = oj_runbook::parse_runbook_with_format(hcl, oj_runbook::Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    super::assert_err_contains(&err, &["agent.w.prime.startup[0]"]);
}

#[test]
fn toml_per_source_prime() {
    let toml = r#"
[agent.w]
run = "claude"

[agent.w.prime]
startup = ["echo startup", "git status"]
resume = "echo resume"
"#;
    match &parse_runbook(toml).unwrap().agents["w"].prime {
        Some(PrimeDef::PerSource(map)) => {
            assert_eq!(map.len(), 2);
            assert!(matches!(map["startup"], PrimeDef::Commands(_)));
            assert!(matches!(map["resume"], PrimeDef::Script(_)));
        }
        other => panic!("expected PerSource, got {:?}", other),
    }
}
