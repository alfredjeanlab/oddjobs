// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn short_form_parses_image() {
    let config: ContainerConfig = serde_json::from_str(r#""coop:claude""#).unwrap();
    assert_eq!(config.image, "coop:claude");
}

#[test]
fn block_form_parses_image() {
    let config: ContainerConfig = serde_json::from_str(r#"{"image": "coop:claude"}"#).unwrap();
    assert_eq!(config.image, "coop:claude");
}

#[test]
fn serializes_as_struct() {
    let config = ContainerConfig::new("coop:claude");
    let json = serde_json::to_string(&config).unwrap();
    assert_eq!(json, r#"{"image":"coop:claude"}"#);
}

#[test]
fn hcl_short_form() {
    let hcl = r#"
        agent "worker" {
            container = "coop:claude"
            run = "claude"
        }
    "#;
    let runbook: crate::Runbook = hcl::from_str(hcl).unwrap();
    let agent = runbook.agents.get("worker").unwrap();
    assert_eq!(agent.container.as_ref().unwrap().image, "coop:claude");
}

#[test]
fn hcl_block_form() {
    let hcl = r#"
        agent "worker" {
            container {
                image = "coop:claude"
            }
            run = "claude"
        }
    "#;
    let runbook: crate::Runbook = hcl::from_str(hcl).unwrap();
    let agent = runbook.agents.get("worker").unwrap();
    assert_eq!(agent.container.as_ref().unwrap().image, "coop:claude");
}

#[test]
fn no_container_is_none() {
    let hcl = r#"
        agent "local" {
            run = "claude"
        }
    "#;
    let runbook: crate::Runbook = hcl::from_str(hcl).unwrap();
    let agent = runbook.agents.get("local").unwrap();
    assert!(agent.container.is_none());
}

#[test]
fn job_container_short_form() {
    let hcl = r#"
        agent "worker" {
            run = "claude"
        }
        job "fix" {
            container = "coop:claude"
            step "fix" {
                run = { agent = "worker" }
            }
        }
    "#;
    let runbook = crate::parse_runbook_with_format(hcl, crate::Format::Hcl).unwrap();
    let job = runbook.jobs.get("fix").unwrap();
    assert_eq!(job.container.as_ref().unwrap().image, "coop:claude");
}
