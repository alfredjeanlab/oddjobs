//! Project setup specs
//!
//! Verify project structure with .oj/runbooks works.

use crate::prelude::*;

#[test]
fn project_with_runbook_directory_is_valid() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);

    // Verify .oj/runbooks exists
    assert!(temp.path().join(".oj/runbooks").is_dir());

    // Verify runbook file exists
    assert!(temp.path().join(".oj/runbooks/build.toml").is_file());
}

#[test]
fn project_with_git_init_has_git_dir() {
    let temp = Project::empty();
    temp.git_init();

    assert!(temp.path().join(".git").is_dir());
}
