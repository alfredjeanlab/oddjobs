// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! oj-core: Core library for the Odd Jobs (oj) CLI tool

pub mod action_tracker;
pub mod agent;
pub mod agent_run;
pub mod clock;
pub mod decision;
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

pub use action_tracker::{ActionTracker, AgentSignal};
pub use agent::{AgentError, AgentId, AgentState};
pub use agent_run::{AgentRun, AgentRunId, AgentRunStatus};
pub use clock::{Clock, FakeClock, SystemClock};
pub use decision::{Decision, DecisionId, DecisionOption, DecisionSource};
pub use effect::Effect;
pub use event::{AgentSignalKind, Event, PromptType, QuestionData, QuestionEntry, QuestionOption};
pub use id::{IdGen, SequentialIdGen, ShortId, UuidIdGen};
pub use namespace::{scoped_name, split_scoped_name};
pub use pipeline::{Pipeline, PipelineConfig, PipelineId, StepOutcome, StepRecord, StepStatus};
pub use session::SessionId;
pub use timer::TimerId;
pub use traced::TracedEffect;
pub use worker::WorkerId;
pub use workspace::{WorkspaceId, WorkspaceStatus};
