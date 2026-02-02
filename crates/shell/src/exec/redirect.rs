// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Redirection setup for file I/O, here-documents, and here-strings.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::{DupTarget, Redirection, Span};

use super::error::ExecError;
use super::expand;
use super::run::ExecContext;

/// Result of processing redirections — may include data to pipe into stdin.
pub(crate) struct AppliedRedirections {
    /// Data to write to the child's stdin pipe after spawning (for heredoc /
    /// herestring). When `Some`, the caller must set `stdin(Stdio::piped())`
    /// before spawning if not already done by this module.
    pub(crate) stdin_data: Option<Vec<u8>>,
}

// ---------------------------------------------------------------------------
// State tracking for fd duplication support
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum StdioConfig {
    /// Use the default set by the caller (typically piped for capture).
    Default,
    /// Redirect to / from a file.
    File { path: String, append: bool },
    /// Redirect to /dev/null (used for close).
    Null,
    /// Pipe for heredoc / herestring data (stdin only).
    PipedData,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Process a list of [`Redirection`]s, applying them to a
/// [`tokio::process::Command`].  Returns any stdin data that must be written
/// after spawning (for heredoc / herestring).
pub(crate) async fn apply_redirections(
    cmd: &mut tokio::process::Command,
    redirections: &[Redirection],
    ctx: &mut ExecContext,
) -> Result<AppliedRedirections, ExecError> {
    let mut stdin_cfg = StdioConfig::Default;
    let mut stdout_cfg = StdioConfig::Default;
    let mut stderr_cfg = StdioConfig::Default;
    let mut stdin_data: Option<Vec<u8>> = None;

    for redir in redirections {
        match redir {
            // ----------------------------------------------------------------
            // Output: > / >>
            // ----------------------------------------------------------------
            Redirection::Out { fd, target, append } => {
                let path = expand::expand_word(ctx, target).await?;
                let cfg = StdioConfig::File {
                    path,
                    append: *append,
                };
                match fd.unwrap_or(1) {
                    1 => stdout_cfg = cfg,
                    2 => stderr_cfg = cfg,
                    _ => {
                        return Err(ExecError::Unsupported {
                            feature: format!("redirection to fd {}", fd.unwrap_or(1)),
                            span: target.span,
                        });
                    }
                }
            }

            // ----------------------------------------------------------------
            // Input: <
            // ----------------------------------------------------------------
            Redirection::In { fd, source } => {
                let path = expand::expand_word(ctx, source).await?;
                match fd.unwrap_or(0) {
                    0 => {
                        stdin_cfg = StdioConfig::File {
                            path,
                            append: false,
                        };
                        stdin_data = None; // file overrides heredoc
                    }
                    _ => {
                        return Err(ExecError::Unsupported {
                            feature: format!("input redirection to fd {}", fd.unwrap_or(0)),
                            span: source.span,
                        });
                    }
                }
            }

            // ----------------------------------------------------------------
            // HereDoc: << / <<-
            // ----------------------------------------------------------------
            Redirection::HereDoc {
                body,
                strip_tabs,
                quoted,
                ..
            } => {
                // Apply tab stripping first if <<-
                let stripped = if *strip_tabs {
                    body.lines()
                        .map(|line| line.strip_prefix('\t').unwrap_or(line))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    body.clone()
                };

                // Expand variables only if delimiter was unquoted
                let content = if *quoted {
                    stripped
                } else {
                    expand::expand_heredoc_body(ctx, &stripped).await?
                };

                stdin_cfg = StdioConfig::PipedData;
                stdin_data = Some(content.into_bytes());
            }

            // ----------------------------------------------------------------
            // HereString: <<<
            // ----------------------------------------------------------------
            Redirection::HereString { content, .. } => {
                let expanded = expand::expand_word(ctx, content).await?;
                // Here-strings append a trailing newline.
                let data = format!("{expanded}\n");
                stdin_cfg = StdioConfig::PipedData;
                stdin_data = Some(data.into_bytes());
            }

            // ----------------------------------------------------------------
            // Both: &> / &>>
            // ----------------------------------------------------------------
            Redirection::Both { append, target } => {
                let path = expand::expand_word(ctx, target).await?;
                let cfg = StdioConfig::File {
                    path,
                    append: *append,
                };
                stdout_cfg = cfg.clone();
                stderr_cfg = cfg;
            }

            // ----------------------------------------------------------------
            // Duplicate: n>&m / n<&m / n>&- / n<&-
            // ----------------------------------------------------------------
            Redirection::Duplicate {
                source,
                target,
                output,
            } => {
                match target {
                    DupTarget::Close => {
                        let null = StdioConfig::Null;
                        match source {
                            0 => stdin_cfg = null,
                            1 => stdout_cfg = null,
                            2 => stderr_cfg = null,
                            _ => {}
                        }
                    }
                    DupTarget::Fd(dest_fd) => {
                        // Copy the current config of dest_fd to source.
                        // e.g. `2>&1` copies stdout config → stderr.
                        let src_config = match dest_fd {
                            0 => stdin_cfg.clone(),
                            1 => stdout_cfg.clone(),
                            2 => stderr_cfg.clone(),
                            _ => StdioConfig::Default,
                        };
                        if *output {
                            match source {
                                1 => stdout_cfg = src_config,
                                2 => stderr_cfg = src_config,
                                _ => {}
                            }
                        } else if *source == 0 {
                            stdin_cfg = src_config;
                        }
                    }
                }
            }
        }
    }

    // Apply the final stdio configs to the Command.
    apply_stdin(cmd, &stdin_cfg, &ctx.cwd, Span::default())?;

    // Special handling for &> redirections: stdout and stderr share the same file.
    // We must open the file once and clone the handle, otherwise the second open
    // with truncate will erase the first write.
    match (&stdout_cfg, &stderr_cfg) {
        (
            StdioConfig::File {
                path: path1,
                append: append1,
            },
            StdioConfig::File {
                path: path2,
                append: append2,
            },
        ) if path1 == path2 && append1 == append2 => {
            // Same file for both stdout and stderr
            let file = open_write(path1, *append1, &ctx.cwd, Span::default())?;
            let file_clone = file
                .try_clone()
                .map_err(|source| ExecError::RedirectFailed {
                    message: format!("cannot clone file handle for '{path1}'"),
                    source,
                    span: Span::default(),
                })?;
            cmd.stdout(std::process::Stdio::from(file));
            cmd.stderr(std::process::Stdio::from(file_clone));
        }
        _ => {
            // Different configs for stdout and stderr
            apply_output(cmd, 1, &stdout_cfg, &ctx.cwd, Span::default())?;
            apply_output(cmd, 2, &stderr_cfg, &ctx.cwd, Span::default())?;
        }
    }

    Ok(AppliedRedirections { stdin_data })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn apply_stdin(
    cmd: &mut tokio::process::Command,
    cfg: &StdioConfig,
    cwd: &Path,
    span: Span,
) -> Result<(), ExecError> {
    match cfg {
        StdioConfig::Default => {} // caller's default
        StdioConfig::File { path, .. } => {
            let file = open_read(path, cwd, span)?;
            cmd.stdin(std::process::Stdio::from(file));
        }
        StdioConfig::Null => {
            cmd.stdin(std::process::Stdio::null());
        }
        StdioConfig::PipedData => {
            cmd.stdin(std::process::Stdio::piped());
        }
    }
    Ok(())
}

fn apply_output(
    cmd: &mut tokio::process::Command,
    fd: u32,
    cfg: &StdioConfig,
    cwd: &Path,
    span: Span,
) -> Result<(), ExecError> {
    match cfg {
        StdioConfig::Default => {} // caller's default (piped for capture)
        StdioConfig::File { path, append } => {
            let file = open_write(path, *append, cwd, span)?;
            let stdio = std::process::Stdio::from(file);
            match fd {
                1 => {
                    cmd.stdout(stdio);
                }
                2 => {
                    cmd.stderr(stdio);
                }
                _ => {}
            }
        }
        StdioConfig::Null => {
            let stdio = std::process::Stdio::null();
            match fd {
                1 => {
                    cmd.stdout(stdio);
                }
                2 => {
                    cmd.stderr(stdio);
                }
                _ => {}
            }
        }
        StdioConfig::PipedData => {} // only relevant for stdin
    }
    Ok(())
}

/// Resolve a path relative to cwd if it's not absolute.
fn resolve_path(path: &str, cwd: &Path) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

fn open_read(path: &str, cwd: &Path, span: Span) -> Result<File, ExecError> {
    let resolved = resolve_path(path, cwd);
    File::open(&resolved).map_err(|source| ExecError::RedirectFailed {
        message: format!("cannot open '{path}' for reading"),
        source,
        span,
    })
}

fn open_write(path: &str, append: bool, cwd: &Path, span: Span) -> Result<File, ExecError> {
    let resolved = resolve_path(path, cwd);
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(!append)
        .append(append)
        .open(&resolved)
        .map_err(|source| ExecError::RedirectFailed {
            message: format!(
                "cannot open '{path}' for {}",
                if append { "appending" } else { "writing" }
            ),
            source,
            span,
        })
}

// ---------------------------------------------------------------------------
// Post-execution redirections (for subshells / brace groups)
// ---------------------------------------------------------------------------

/// Result of applying captured redirections.
pub(crate) struct CapturedRedirectionResult {
    /// True if stdout was redirected to a file.
    pub(crate) stdout_redirected: bool,
    /// True if stderr was redirected to a file.
    pub(crate) stderr_redirected: bool,
}

/// Apply output redirections to already-captured stdout/stderr data.
///
/// Used for subshells and brace groups where we need to redirect the
/// collected output of the body to files.
pub(crate) async fn apply_captured_redirections(
    redirections: &[Redirection],
    stdout: &str,
    stderr: &str,
    ctx: &mut ExecContext,
) -> Result<CapturedRedirectionResult, ExecError> {
    use std::io::Write;

    let mut stdout_redirected = false;
    let mut stderr_redirected = false;

    for redir in redirections {
        match redir {
            Redirection::Out { fd, target, append } => {
                let path = expand::expand_word(ctx, target).await?;
                let span = target.span;
                match fd.unwrap_or(1) {
                    1 => {
                        let mut file = open_write(&path, *append, &ctx.cwd, span)?;
                        file.write_all(stdout.as_bytes()).map_err(|source| {
                            ExecError::RedirectFailed {
                                message: format!("cannot write to '{path}'"),
                                source,
                                span,
                            }
                        })?;
                        stdout_redirected = true;
                    }
                    2 => {
                        let mut file = open_write(&path, *append, &ctx.cwd, span)?;
                        file.write_all(stderr.as_bytes()).map_err(|source| {
                            ExecError::RedirectFailed {
                                message: format!("cannot write to '{path}'"),
                                source,
                                span,
                            }
                        })?;
                        stderr_redirected = true;
                    }
                    _ => {
                        return Err(ExecError::Unsupported {
                            feature: format!("redirection to fd {}", fd.unwrap_or(1)),
                            span,
                        });
                    }
                }
            }
            Redirection::Both { append, target } => {
                let path = expand::expand_word(ctx, target).await?;
                let span = target.span;
                let mut file = open_write(&path, *append, &ctx.cwd, span)?;
                file.write_all(stdout.as_bytes())
                    .map_err(|source| ExecError::RedirectFailed {
                        message: format!("cannot write to '{path}'"),
                        source,
                        span,
                    })?;
                file.write_all(stderr.as_bytes())
                    .map_err(|source| ExecError::RedirectFailed {
                        message: format!("cannot write to '{path}'"),
                        source,
                        span,
                    })?;
                stdout_redirected = true;
                stderr_redirected = true;
            }
            // Input redirections don't apply to captured output
            Redirection::In { .. }
            | Redirection::HereDoc { .. }
            | Redirection::HereString { .. }
            | Redirection::Duplicate { .. } => {}
        }
    }

    Ok(CapturedRedirectionResult {
        stdout_redirected,
        stderr_redirected,
    })
}
