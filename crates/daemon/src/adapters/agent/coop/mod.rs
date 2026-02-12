// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Coop agent adapter implementation
//!
//! Manages Claude Code agents via the coop process.
//! Each agent runs inside a coop-managed PTY, with state detection, input handling, and
//! lifecycle management accessible via coop's HTTP API over Unix sockets.

pub(crate) mod adapter;
pub(crate) mod http;
mod spawn;
pub(crate) mod ws;

pub use adapter::LocalAdapter;
