// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tracing infrastructure for effects and operations

/// Trait for operations that should be traced
///
/// Provides consistent naming and structured fields for logging.
pub trait TracedEffect {
    /// Effect name for log spans (e.g., "spawn", "worktree_add")
    fn name(&self) -> &'static str;

    /// Key-value pairs for structured logging
    fn fields(&self) -> Vec<(&'static str, String)>;
}
