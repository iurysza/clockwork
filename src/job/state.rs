use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::run_record::LastRun;

use super::name::JobName;

/// Whether the scheduler may claim new runs. Stored separately from the definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Disabled,
    Enabled,
}

impl std::fmt::Display for Activation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "disabled"),
            Self::Enabled => write!(f, "enabled"),
        }
    }
}

/// Public job state derived from its source and runtime data.
/// Inspection rejects contradictory data with `IntegrityViolation`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManagedJobState {
    /// Valid source without a runtime job, e.g. an interrupted create.
    Draft { source_revision: String },
    Disabled {
        source_revision: String,
        runtime_generation: u32,
    },
    /// Always carries a future `next_run`; never exposed with a past one.
    Scheduled {
        source_revision: String,
        runtime_generation: u32,
        next_run: DateTime<Utc>,
    },
    Running {
        source_revision: String,
        runtime_generation: u32,
        run_id: String,
        scheduled_for: DateTime<Utc>,
    },
    Completed {
        source_revision: String,
        runtime_generation: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_run: Option<LastRun>,
    },
}

impl ManagedJobState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Draft { .. } => "draft",
            Self::Disabled { .. } => "disabled",
            Self::Scheduled { .. } => "scheduled",
            Self::Running { .. } => "running",
            Self::Completed { .. } => "completed",
        }
    }

    pub fn source_revision(&self) -> &str {
        match self {
            Self::Draft { source_revision }
            | Self::Disabled {
                source_revision, ..
            }
            | Self::Scheduled {
                source_revision, ..
            }
            | Self::Running {
                source_revision, ..
            }
            | Self::Completed {
                source_revision, ..
            } => source_revision,
        }
    }

    pub fn next_run(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Scheduled { next_run, .. } => Some(*next_run),
            _ => None,
        }
    }
}

/// Revision of the source, runtime job, and referenced profile.
/// `--if-revision` rejects mutations when any of these has changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateRevision {
    pub source: Option<String>,
    pub runtime: Option<String>,
    /// Resolved profile revision, so profile edits invalidate a job preview.
    pub profile: Option<String>,
}

impl StateRevision {
    pub fn combined(&self) -> String {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(self.source.as_deref().unwrap_or("<none>"));
        hasher.update(b"\0");
        hasher.update(self.runtime.as_deref().unwrap_or("<none>"));
        hasher.update(b"\0");
        hasher.update(self.profile.as_deref().unwrap_or("<none>"));
        format!("rev_{}", hex_digest(&hasher.finalize()))
    }
}

pub fn content_revision(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("rev_{}", hex_digest(&hasher.finalize()))
}

fn hex_digest(digest: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The public view for one managed job.
#[derive(Debug, Clone)]
pub struct JobView {
    pub name: JobName,
    pub state: ManagedJobState,
    pub revision: StateRevision,
    pub schedule_input: String,
    pub action_kind: super::definition::ActionKind,
    pub tags: Vec<String>,
    pub activation: Activation,
}

/// Per-job validation result for `clockwork job validate`.
#[derive(Debug, Serialize)]
pub struct JobValidation {
    pub job: String,
    pub valid: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub ok: bool,
    pub jobs: Vec<JobValidation>,
}
