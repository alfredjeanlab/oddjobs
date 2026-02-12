// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! DTO structs for the IPC protocol.

use std::collections::HashMap;
use std::path::PathBuf;

use oj_core::{
    Crew, Decision, DecisionOption, Job, OwnerId, StepOutcome, StepOutcomeKind, StepRecord,
    StepStatusKind,
};
use oj_storage::{QueueItem, WorkerRecord, Workspace};
use serde::{Deserialize, Serialize};

/// Summary of a job for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub step: String,
    pub step_status: StepStatusKind,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub project: String,
    pub retries: u32,
}

/// Detailed job information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobDetail {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub step: String,
    pub step_status: StepStatusKind,
    pub vars: HashMap<String, String>,
    pub workspace_path: Option<PathBuf>,
    pub error: Option<String>,
    pub steps: Vec<StepRecordDetail>,
    pub agents: Vec<AgentSummary>,
    pub project: String,
}

/// Record of a step execution for display
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepRecordDetail {
    pub name: String,
    pub outcome: StepOutcomeKind,
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

impl From<&StepRecord> for StepRecordDetail {
    fn from(r: &StepRecord) -> Self {
        StepRecordDetail {
            name: r.name.clone(),
            started_at_ms: r.started_at_ms,
            finished_at_ms: r.finished_at_ms,
            outcome: StepOutcomeKind::from(&r.outcome),
            detail: match &r.outcome {
                StepOutcome::Failed(e) => Some(e.clone()),
                StepOutcome::Waiting(r) => Some(r.clone()),
                _ => None,
            },
            agent_id: r.agent_id.clone(),
            agent_name: r.agent_name.clone(),
        }
    }
}

/// Detailed agent information for `oj agent show`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentDetail {
    pub agent_id: String,
    pub agent_name: Option<String>,
    #[serde(default)]
    pub crew_id: String,
    pub job_id: String,
    pub job_name: String,
    pub step_name: String,
    pub project: String,
    pub status: String,
    pub workspace_path: Option<PathBuf>,
    pub files_read: usize,
    pub files_written: usize,
    pub commands_run: usize,
    pub exit_reason: Option<String>,
    pub error: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub updated_at_ms: u64,
}

/// Summary of agent activity for a job step
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSummary {
    /// Job that owns this agent
    #[serde(default)]
    pub job_id: String,
    /// Crew invocation ID (for standalone agents)
    #[serde(default)]
    pub crew_id: String,
    /// Step name that spawned this agent
    pub step_name: String,
    /// Agent instance ID
    pub agent_id: String,
    /// Agent name from the runbook definition
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// Project project
    #[serde(default)]
    pub project: String,
    /// Current status: "completed", "running", "failed", "waiting"
    pub status: String,
    /// Number of files read
    pub files_read: usize,
    /// Number of files written or edited
    pub files_written: usize,
    /// Number of commands run
    pub commands_run: usize,
    /// Exit reason (e.g. "completed", "idle (gate passed)", "failed: ...")
    pub exit_reason: Option<String>,
    /// Most recent activity timestamp (from step history)
    #[serde(default)]
    pub updated_at_ms: u64,
}

/// Summary of a workspace for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceSummary {
    pub id: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub status: String,
    pub project: String,
    pub created_at_ms: u64,
}

/// Detailed workspace information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceDetail {
    pub id: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub owner: String,
    pub status: String,
    pub created_at_ms: u64,
}

/// Workspace entry for drop/prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceEntry {
    pub id: String,
    pub path: PathBuf,
    pub branch: Option<String>,
}

/// Summary of a queue item
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueItemSummary {
    pub id: String,
    pub status: String,
    pub data: HashMap<String, String>,
    pub worker_name: Option<String>,
    pub pushed_at_ms: u64,
    pub failures: u32,
}

/// Summary of a queue for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueSummary {
    pub name: String,
    pub project: String,
    pub queue_type: String,
    pub item_count: usize,
    pub workers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_poll_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_polled_at_ms: Option<u64>,
}

/// Summary of a decision for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionSummary {
    pub id: String,
    pub owner_id: String,
    pub owner_name: String,
    pub project: String,
    pub source: String,
    pub summary: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionDetail {
    pub id: String,
    pub owner_id: String,
    pub owner_name: String,
    pub project: String,
    pub agent_id: String,
    pub source: String,
    pub context: String,
    pub options: Vec<DecisionOptionDetail>,

    /// Per-question 1-indexed answers for multi-question decisions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<usize>,
    /// Grouped question data for multi-question decisions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub question_groups: Vec<QuestionGroupDetail>,

    pub message: Option<String>,
    pub created_at_ms: u64,
    pub resolved_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

/// A question group for multi-question decisions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuestionGroupDetail {
    pub question: String,
    pub options: Vec<DecisionOptionDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
}

/// A single decision option for display
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionOptionDetail {
    pub number: usize,
    pub label: String,
    pub description: Option<String>,
    pub recommended: bool,
}

/// Summary of a worker for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerSummary {
    pub name: String,
    pub project: String,
    pub queue: String,
    pub status: String,
    pub active: usize,
    pub concurrency: u32,
    /// Most recent activity timestamp (from active jobs)
    pub updated_at_ms: u64,
}

impl From<&Job> for JobSummary {
    fn from(p: &Job) -> Self {
        let created_at_ms = p.step_history.first().map(|r| r.started_at_ms).unwrap_or(0);
        let updated_at_ms =
            p.step_history.last().map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms)).unwrap_or(0);
        JobSummary {
            id: p.id.clone(),
            name: p.name.clone(),
            kind: p.kind.clone(),
            step: p.step.clone(),
            step_status: StepStatusKind::from(&p.step_status),
            created_at_ms,
            updated_at_ms,
            project: p.project.clone(),
            retries: p.total_retries,
        }
    }
}

impl From<&QueueItem> for QueueItemSummary {
    fn from(item: &QueueItem) -> Self {
        QueueItemSummary {
            id: item.id.clone(),
            status: item.status.to_string(),
            data: item.data.clone(),
            worker_name: item.worker.clone(),
            pushed_at_ms: item.pushed_at_ms,
            failures: item.failures,
        }
    }
}

impl From<&Workspace> for WorkspaceEntry {
    fn from(w: &Workspace) -> Self {
        WorkspaceEntry { id: w.id.clone(), path: w.path.clone(), branch: w.branch.clone() }
    }
}

impl From<&Workspace> for WorkspaceDetail {
    fn from(w: &Workspace) -> Self {
        let owner = match &w.owner {
            OwnerId::Job(job_id) => job_id.to_string(),
            OwnerId::Crew(run_id) => run_id.to_string(),
        };
        WorkspaceDetail {
            id: w.id.clone(),
            path: w.path.clone(),
            branch: w.branch.clone(),
            owner,
            status: w.status.to_string(),
            created_at_ms: w.created_at_ms,
        }
    }
}

impl WorkerSummary {
    pub fn from_worker(w: &WorkerRecord, updated_at_ms: u64) -> Self {
        WorkerSummary {
            name: w.name.clone(),
            project: w.project.clone(),
            queue: w.queue.clone(),
            status: w.status.clone(),
            active: w.active.len(),
            concurrency: w.concurrency,
            updated_at_ms,
        }
    }
}

impl DecisionSummary {
    pub fn from_decision(d: &Decision, owner_name: String) -> Self {
        let summary = if d.context.len() > 80 {
            format!("{}...", &d.context[..77])
        } else {
            d.context.clone()
        };
        DecisionSummary {
            id: d.id.to_string(),
            owner_id: d.owner.to_string(),
            owner_name,
            source: format!("{:?}", d.source).to_lowercase(),
            summary,
            created_at_ms: d.created_at_ms,
            project: d.project.clone(),
        }
    }
}

impl DecisionOptionDetail {
    pub fn from_option(index: usize, opt: &DecisionOption) -> Self {
        DecisionOptionDetail {
            number: index + 1,
            label: opt.label.clone(),
            description: opt.description.clone(),
            recommended: opt.recommended,
        }
    }
}

impl DecisionDetail {
    pub fn from_decision(d: &Decision, owner_name: String) -> Self {
        let options: Vec<DecisionOptionDetail> = d
            .options
            .iter()
            .enumerate()
            .map(|(i, opt)| DecisionOptionDetail::from_option(i, opt))
            .collect();

        let question_groups = d
            .questions
            .as_ref()
            .map(|qd| {
                qd.questions
                    .iter()
                    .map(|entry| {
                        let mut opts: Vec<DecisionOptionDetail> = entry
                            .options
                            .iter()
                            .enumerate()
                            .map(|(i, opt)| DecisionOptionDetail {
                                number: i + 1,
                                label: opt.label.clone(),
                                description: opt.description.clone(),
                                recommended: false,
                            })
                            .collect();
                        opts.push(DecisionOptionDetail {
                            number: opts.len() + 1,
                            label: "Other".to_string(),
                            description: Some("Write a custom response".to_string()),
                            recommended: false,
                        });
                        QuestionGroupDetail {
                            question: entry.question.clone(),
                            header: entry.header.clone(),
                            options: opts,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        DecisionDetail {
            id: d.id.to_string(),
            owner_id: d.owner.to_string(),
            owner_name,
            agent_id: d.agent_id.to_string(),
            source: format!("{:?}", d.source).to_lowercase(),
            context: d.context.clone(),
            options,
            question_groups,
            choices: d.choices.clone(),
            message: d.message.clone(),
            created_at_ms: d.created_at_ms,
            resolved_at_ms: d.resolved_at_ms,
            superseded_by: d.superseded_by.as_ref().map(|id| id.to_string()),
            project: d.project.clone(),
        }
    }
}

impl AgentDetail {
    pub fn from_summary(
        summary: &AgentSummary,
        job: &Job,
        error: Option<String>,
        started_at_ms: u64,
        finished_at_ms: Option<u64>,
    ) -> Self {
        AgentDetail {
            agent_id: summary.agent_id.clone(),
            agent_name: summary.agent_name.clone(),
            crew_id: String::new(),
            job_id: job.id.clone(),
            job_name: job.name.clone(),
            step_name: summary.step_name.clone(),
            project: summary.project.clone(),
            status: summary.status.clone(),
            workspace_path: job.workspace_path.clone(),
            files_read: summary.files_read,
            files_written: summary.files_written,
            commands_run: summary.commands_run,
            exit_reason: summary.exit_reason.clone(),
            error,
            started_at_ms,
            finished_at_ms,
            updated_at_ms: summary.updated_at_ms,
        }
    }
}

impl From<&Crew> for AgentDetail {
    fn from(crew: &Crew) -> Self {
        AgentDetail {
            agent_id: crew.agent_id.clone().unwrap_or_default(),
            agent_name: Some(crew.agent_name.clone()),
            crew_id: crew.id.clone(),
            job_id: String::new(),
            job_name: crew.command_name.clone(),
            step_name: String::new(),
            project: crew.project.clone(),
            status: crew.status.to_string(),
            workspace_path: Some(crew.cwd.clone()),
            files_read: 0,
            files_written: 0,
            commands_run: 0,
            exit_reason: crew.error.clone(),
            error: crew.error.clone(),
            started_at_ms: crew.created_at_ms,
            finished_at_ms: None,
            updated_at_ms: crew.updated_at_ms,
        }
    }
}

impl From<&Crew> for AgentSummary {
    fn from(crew: &Crew) -> Self {
        AgentSummary {
            job_id: String::new(),
            crew_id: crew.id.clone(),
            step_name: String::new(),
            agent_id: crew.agent_id.clone().unwrap_or_default(),
            agent_name: Some(crew.agent_name.clone()),
            project: crew.project.clone(),
            status: crew.status.to_string(),
            files_read: 0,
            files_written: 0,
            commands_run: 0,
            exit_reason: crew.error.clone(),
            updated_at_ms: crew.updated_at_ms,
        }
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
