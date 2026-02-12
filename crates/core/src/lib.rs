// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! oj-core: Core library for the Odd Jobs (oj) CLI tool

pub mod macros;

pub mod actions;
pub mod agent;
pub mod agent_record;
pub mod clock;
pub mod container;
pub mod crew;
pub mod decision;
pub mod effect;
pub mod event;
pub mod id;
pub mod job;
pub mod owner;
pub mod project;
pub mod target;
pub mod time_fmt;
pub mod timer;
pub mod worker;
pub mod workspace;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

// ActionTracker available via actions module or job re-export
pub use agent::{agent_dir, AgentError, AgentId, AgentState, PromptResponse};
pub use agent_record::{AgentRecord, AgentRecordStatus};
pub use clock::{Clock, FakeClock, SystemClock};
pub use container::ContainerConfig;
#[cfg(any(test, feature = "test-support"))]
pub use crew::CrewBuilder;
pub use crew::{Crew, CrewId, CrewStatus};
pub use decision::{Decision, DecisionId, DecisionOption, DecisionSource};
pub use effect::Effect;
pub use event::{Event, PromptType, QuestionData, QuestionEntry, QuestionOption};
pub use id::{short, IdGen, UuidIdGen};
#[cfg(any(test, feature = "test-support"))]
pub use job::JobBuilder;
pub use job::{
    Job, JobConfig, JobConfigBuilder, JobId, StepOutcome, StepOutcomeKind, StepRecord, StepStatus,
    StepStatusKind,
};
pub use owner::{OwnerId, OwnerMismatch};
pub use project::{namespace_to_option, scoped_name, split_scoped_name, Namespace};
pub use target::RunTarget;
pub use time_fmt::{format_elapsed, format_elapsed_ms};
pub use timer::{TimerId, TimerKind};
// WorkerId available via worker module if needed
pub use workspace::{WorkspaceId, WorkspaceStatus};
