use super::*;

#[test]
fn display_roundtrip() {
    let cases = vec![
        RunTarget::Job("build".into()),
        RunTarget::Agent("planning".into()),
        RunTarget::Shell("echo hello".into()),
    ];
    for target in cases {
        let s = target.to_string();
        let parsed: RunTarget = s.parse().unwrap();
        assert_eq!(parsed, target);
    }
}

#[test]
fn bare_name_parses_as_job() {
    let t: RunTarget = "build".parse().unwrap();
    assert_eq!(t, RunTarget::Job("build".into()));
}

#[test]
fn serde_roundtrip() {
    let target = RunTarget::Agent("planner".into());
    let json = serde_json::to_string(&target).unwrap();
    assert_eq!(json, "\"agent:planner\"");
    let parsed: RunTarget = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, target);
}

#[test]
fn helpers() {
    let j = RunTarget::job("build");
    assert!(j.is_job());
    assert!(!j.is_agent());
    assert_eq!(j.name(), "build");

    let a = RunTarget::agent("planner");
    assert!(a.is_agent());
    assert_eq!(a.name(), "planner");

    let s = RunTarget::shell("echo hi");
    assert!(s.is_shell());
    assert_eq!(s.name(), "echo hi");
}
