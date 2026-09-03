use chrono::{DateTime, Utc};
use serde::Serialize;

use super::name::JobName;

/// Typed failures for the managed job application layer. Every variant maps
/// to a stable `CW_*` code and can suggest one safe recovery command.
/// Pre-mutation failures report no change; `MutationFailed` reports that an
/// earlier step reached durable storage.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("{message}")]
    InvalidInput { message: String },

    #[error("job '{0}' not found")]
    JobNotFound(#[from] NotFound),

    #[error("job '{0}' already exists")]
    JobAlreadyExists(JobName),

    #[error("{message}")]
    IllegalTransition {
        job: JobName,
        current_state: &'static str,
        operation: &'static str,
        message: String,
        recovery: Option<String>,
    },

    #[error("state changed after inspection: expected revision {expected}, found {actual}")]
    RevisionConflict {
        job: Option<JobName>,
        expected: String,
        actual: String,
    },

    #[error("job '{job}' has a run in flight (run {run_id})")]
    RunInFlight {
        job: JobName,
        run_id: String,
        scheduled_for: DateTime<Utc>,
    },

    #[error("{message}")]
    IntegrityViolation {
        job: Option<JobName>,
        message: String,
    },

    #[error("job source failure: {message}")]
    SourceFailure { message: String },

    #[error("runtime state failure: {message}")]
    RuntimeFailure { message: String },

    #[error("job '{job}' postcondition failed: expected {expected}, found {actual}")]
    PostconditionFailed {
        job: JobName,
        expected: String,
        actual: String,
    },

    #[error("{operation} for job '{job}' stopped after changing state: {message}")]
    MutationFailed {
        job: JobName,
        operation: &'static str,
        message: String,
    },
}

/// Newtype so `JobName` lookups can return a typed not-found error through
/// `?` without conflicting with other `From` impls.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct NotFound(pub JobName);

impl JobError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput {
            message: message.into(),
        }
    }

    pub fn illegal_transition(
        job: JobName,
        current_state: &'static str,
        operation: &'static str,
        message: impl Into<String>,
        recovery: Option<String>,
    ) -> Self {
        Self::IllegalTransition {
            job,
            current_state,
            operation,
            message: message.into(),
            recovery,
        }
    }

    pub fn integrity(job: Option<JobName>, message: impl Into<String>) -> Self {
        Self::IntegrityViolation {
            job,
            message: message.into(),
        }
    }

    /// Stable machine-readable code for JSON output and tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput { .. } => "CW_INVALID_INPUT",
            Self::JobNotFound(_) => "CW_JOB_NOT_FOUND",
            Self::JobAlreadyExists(_) => "CW_JOB_ALREADY_EXISTS",
            Self::IllegalTransition { .. } => "CW_ILLEGAL_TRANSITION",
            Self::RevisionConflict { .. } => "CW_REVISION_CONFLICT",
            Self::RunInFlight { .. } => "CW_RUN_IN_FLIGHT",
            Self::IntegrityViolation { .. } => "CW_INTEGRITY_VIOLATION",
            Self::SourceFailure { .. } => "CW_SOURCE_FAILURE",
            Self::RuntimeFailure { .. } => "CW_RUNTIME_FAILURE",
            Self::PostconditionFailed { .. } => "CW_POSTCONDITION_FAILED",
            Self::MutationFailed { .. } => "CW_MUTATION_FAILED",
        }
    }

    /// Planner failures occur before mutation. A coordinator failure reports
    /// the successful earlier steps so callers can safely inspect and retry.
    pub fn changed(&self) -> bool {
        matches!(self, Self::MutationFailed { .. })
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Self::JobNotFound(_) => 3,
            Self::InvalidInput { .. }
            | Self::JobAlreadyExists(_)
            | Self::IllegalTransition { .. } => 2,
            Self::RevisionConflict { .. } => 7,
            Self::RunInFlight { .. } => 8,
            Self::IntegrityViolation { .. } => 9,
            Self::SourceFailure { .. }
            | Self::RuntimeFailure { .. }
            | Self::PostconditionFailed { .. }
            | Self::MutationFailed { .. } => 1,
        }
    }

    pub fn job_name(&self) -> Option<&JobName> {
        match self {
            Self::JobNotFound(found) => Some(&found.0),
            Self::JobAlreadyExists(name)
            | Self::IllegalTransition { job: name, .. }
            | Self::RunInFlight { job: name, .. }
            | Self::PostconditionFailed { job: name, .. }
            | Self::MutationFailed { job: name, .. } => Some(name),
            Self::RevisionConflict { job, .. } | Self::IntegrityViolation { job, .. } => {
                job.as_ref()
            }
            Self::InvalidInput { .. }
            | Self::SourceFailure { .. }
            | Self::RuntimeFailure { .. } => None,
        }
    }

    /// One safe recovery command, when Clockwork knows it.
    pub fn recovery(&self) -> Option<&str> {
        match self {
            Self::IllegalTransition { recovery, .. } => recovery.as_deref(),
            _ => None,
        }
    }

    /// JSON error body for the `--json` envelope.
    pub fn to_json(&self) -> ErrorJson {
        let (current_state, requested_operation) = match self {
            Self::IllegalTransition {
                current_state,
                operation,
                ..
            } => (Some(*current_state), Some(*operation)),
            Self::MutationFailed { operation, .. } => (None, Some(*operation)),
            _ => (None, None),
        };
        ErrorJson {
            code: self.code(),
            job: self.job_name().map(|n| n.as_str().to_string()),
            current_state,
            requested_operation,
            message: self.to_string(),
            recovery: self.recovery().map(|command| RecoveryJson {
                command: command.to_string(),
            }),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RecoveryJson {
    pub command: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorJson {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_operation: Option<&'static str>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryJson>,
}
