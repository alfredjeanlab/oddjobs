// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker identifier.

crate::define_id! {
    /// Unique identifier for a worker instance.
    pub struct WorkerId("wrk-");
}
