// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! oj-core: Core library for the Odd Jobs (oj) CLI tool

pub mod agent;
pub mod clock;
pub mod effect;
pub mod event;
pub mod id;
pub mod namespace;
pub mod pipeline;
pub mod session;
pub mod timer;
pub mod traced;
pub mod worker;
pub mod workspace;

pub use agent::{AgentError, AgentId, AgentState};
pub use clock::{Clock, FakeClock, SystemClock};
pub use effect::Effect;
pub use event::{AgentSignalKind, Event};
pub use id::{IdGen, SequentialIdGen, UuidIdGen};
pub use pipeline::{Pipeline, PipelineConfig, PipelineId, StepOutcome, StepRecord, StepStatus};
pub use session::SessionId;
pub use timer::TimerId;
pub use traced::TracedEffect;
pub use worker::WorkerId;
pub use workspace::{WorkspaceId, WorkspaceStatus};
