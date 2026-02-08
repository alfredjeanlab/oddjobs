// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Workspace filesystem operations (worktree and folder creation).

/// Create a git worktree at the given path.
pub(crate) async fn create_worktree(
    path: &std::path::Path,
    repo_root: Option<std::path::PathBuf>,
    branch: Option<String>,
    start_point: Option<String>,
) -> Result<(), String> {
    // Create parent directory
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create workspace parent dir: {}", e))?;
    }

    let repo_root = repo_root.ok_or("repo_root required for worktree workspace")?;
    let branch = branch.ok_or("branch required for worktree workspace")?;
    let start_point = start_point.unwrap_or_else(|| "HEAD".to_string());

    let path_str = path.display().to_string();
    let mut cmd = tokio::process::Command::new("git");
    cmd.args([
        "-C",
        &repo_root.display().to_string(),
        "worktree",
        "add",
        "-b",
        &branch,
        &path_str,
        &start_point,
    ])
    .env_remove("GIT_DIR")
    .env_remove("GIT_WORK_TREE");
    let output = oj_adapters::subprocess::run_with_timeout(
        cmd,
        oj_adapters::subprocess::GIT_WORKTREE_TIMEOUT,
        "git worktree add",
    )
    .await
    .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree add failed: {}", stderr.trim()));
    }

    Ok(())
}

/// Create a plain directory workspace.
pub(crate) async fn create_folder(path: &std::path::Path) -> Result<(), String> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|e| format!("failed to create workspace dir: {}", e))
}
