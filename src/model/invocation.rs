use chrono::{DateTime, Utc};
use thiserror::Error;

use super::run_record::Trigger;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationSource {
    Scheduled { occurrence_at: DateTime<Utc> },
    Manual,
}

impl InvocationSource {
    pub fn trigger(self) -> Trigger {
        match self {
            Self::Scheduled { .. } => Trigger::Scheduled,
            Self::Manual => Trigger::Manual,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub job_id: String,
    pub run_id: String,
    pub requested_at: DateTime<Utc>,
    pub source: InvocationSource,
}

impl Invocation {
    pub fn scheduled(
        job_id: impl Into<String>,
        run_id: impl Into<String>,
        occurrence_at: DateTime<Utc>,
    ) -> Self {
        Self {
            job_id: job_id.into(),
            run_id: run_id.into(),
            requested_at: occurrence_at,
            source: InvocationSource::Scheduled { occurrence_at },
        }
    }

    pub fn manual(
        job_id: impl Into<String>,
        run_id: impl Into<String>,
        requested_at: DateTime<Utc>,
    ) -> Self {
        Self {
            job_id: job_id.into(),
            run_id: run_id.into(),
            requested_at,
            source: InvocationSource::Manual,
        }
    }

    pub fn recorded_for(&self) -> DateTime<Utc> {
        match self.source {
            InvocationSource::Scheduled { occurrence_at } => occurrence_at,
            InvocationSource::Manual => self.requested_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAttempt {
    pub job_id: String,
    pub run_id: String,
    pub requested_at: DateTime<Utc>,
    pub source: InvocationSource,
}

impl RunAttempt {
    pub fn recorded_for(&self) -> DateTime<Utc> {
        match self.source {
            InvocationSource::Scheduled { occurrence_at } => occurrence_at,
            InvocationSource::Manual => self.requested_at,
        }
    }

    pub fn trigger(&self) -> Trigger {
        self.source.trigger()
    }
}

impl From<&Invocation> for RunAttempt {
    fn from(invocation: &Invocation) -> Self {
        Self {
            job_id: invocation.job_id.clone(),
            run_id: invocation.run_id.clone(),
            requested_at: invocation.requested_at,
            source: invocation.source,
        }
    }
}

#[derive(Debug, Error)]
pub enum InvocationInputError {
    #[error("Missing --run-id for scheduled execution")]
    MissingScheduledRunId,
    #[error("Fallback runs use the dedicated _exec-fallback command")]
    FallbackIsNotPrimary,
}
