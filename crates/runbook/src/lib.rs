// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! Runbook parsing and definition

mod agent;
mod command;
mod find;
mod parser;
mod pipeline;
mod queue;
mod template;
mod worker;

pub use agent::{
    ActionConfig, ActionTrigger, AgentAction, AgentDef, Attempts, ErrorActionConfig, ErrorMatch,
    ErrorType, PrimeDef,
};
pub use command::{
    parse_arg_spec, ArgDef, ArgSpec, ArgSpecError, ArgValidationError, CommandDef, FlagDef,
    OptionDef, RunDirective, VariadicDef,
};
pub use find::{
    collect_all_commands, find_runbook_by_command, find_runbook_by_queue, find_runbook_by_worker,
    FindError,
};
pub use parser::{parse_runbook, parse_runbook_with_format, Format, ParseError, Runbook};
pub use pipeline::{NotifyConfig, PipelineDef, StepDef, StepTransition, WorkspaceMode};
pub use queue::{QueueDef, QueueType};
pub use template::{escape_for_shell, interpolate, interpolate_shell};
pub use worker::{WorkerDef, WorkerHandler, WorkerSource};
