// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! oj-core: Core library for the Odd Jobs (oj) CLI tool

pub mod action_tracker;
pub mod agent;
pub mod agent_record;
pub mod agent_run;
pub mod clock;
pub mod decision;
pub mod effect;
pub mod event;
pub mod id;
pub mod job;
pub mod namespace;
pub mod owner;
pub mod session;
pub mod time_fmt;
pub mod timer;
pub mod worker;
pub mod workspace;

// ActionTracker and AgentSignal available via action_tracker module or job re-export
pub use agent::{AgentError, AgentId, AgentState};
pub use agent_record::{AgentRecord, AgentRecordStatus};
pub use agent_run::{AgentRun, AgentRunId, AgentRunStatus};
pub use clock::{Clock, FakeClock, SystemClock};
pub use decision::{Decision, DecisionId, DecisionOption, DecisionSource};
pub use effect::Effect;
pub use event::{AgentSignalKind, Event, PromptType, QuestionData, QuestionEntry, QuestionOption};
pub use id::{IdGen, ShortId, UuidIdGen};
pub use job::{Job, JobConfig, JobId, StepOutcome, StepRecord, StepStatus};
pub use namespace::{namespace_to_option, scoped_name, split_scoped_name, Namespace};
pub use owner::OwnerId;
pub use session::SessionId;
pub use time_fmt::{format_elapsed, format_elapsed_ms};
pub use timer::TimerId;
// WorkerId available via worker module if needed
pub use workspace::{WorkspaceId, WorkspaceStatus};
