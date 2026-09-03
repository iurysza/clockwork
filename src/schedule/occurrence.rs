use chrono::{DateTime, Duration, Local, Utc};
use thiserror::Error;

use crate::model::schedule::JobSchedule;

#[derive(Debug, Error)]
pub enum OccurrenceError {
    #[error("invalid cron expression '{expression}'")]
    InvalidCron {
        expression: String,
        #[source]
        source: cron::error::Error,
    },
    #[error("recurring interval must be greater than zero")]
    InvalidInterval,
}

/// Return the latest occurrence in `(after, now]`.
pub fn latest_due(
    schedule: &JobSchedule,
    after: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, OccurrenceError> {
    match schedule {
        JobSchedule::RecurringCron { expr } => {
            let schedule = parse_cron(expr)?;
            let after_local = after.with_timezone(&Local);
            let now_local = now.with_timezone(&Local);
            let mut latest = None;

            for occurrence in schedule.after(&after_local) {
                if occurrence > now_local {
                    break;
                }
                latest = Some(occurrence.with_timezone(&Utc));
            }

            Ok(latest)
        }
        JobSchedule::RecurringInterval { every_seconds } => {
            let seconds = interval_seconds(*every_seconds)?;
            let elapsed = (now - after).num_seconds();
            if elapsed < seconds {
                return Ok(None);
            }

            let periods = elapsed / seconds;
            Ok(Some(after + Duration::seconds(periods * seconds)))
        }
        JobSchedule::OneShot { fire_at } => {
            if *fire_at > after && *fire_at <= now {
                Ok(Some(*fire_at))
            } else {
                Ok(None)
            }
        }
    }
}

/// Return every occurrence in `(after, now]`.
pub fn due_after(
    schedule: &JobSchedule,
    after: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Vec<DateTime<Utc>>, OccurrenceError> {
    match schedule {
        JobSchedule::RecurringCron { expr } => {
            let schedule = parse_cron(expr)?;
            let after_local = after.with_timezone(&Local);
            let now_local = now.with_timezone(&Local);
            Ok(schedule
                .after(&after_local)
                .take_while(|occurrence| *occurrence <= now_local)
                .map(|occurrence| occurrence.with_timezone(&Utc))
                .collect())
        }
        JobSchedule::RecurringInterval { every_seconds } => {
            let seconds = interval_seconds(*every_seconds)?;
            let mut occurrences = Vec::new();
            let mut next = after + Duration::seconds(seconds);
            while next <= now {
                occurrences.push(next);
                next += Duration::seconds(seconds);
            }
            Ok(occurrences)
        }
        JobSchedule::OneShot { fire_at } => {
            if *fire_at > after && *fire_at <= now {
                Ok(vec![*fire_at])
            } else {
                Ok(Vec::new())
            }
        }
    }
}

/// Return the first occurrence after `after`.
pub fn next_after(
    schedule: &JobSchedule,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, OccurrenceError> {
    match schedule {
        JobSchedule::RecurringCron { expr } => {
            let schedule = parse_cron(expr)?;
            let after_local = after.with_timezone(&Local);
            Ok(schedule
                .after(&after_local)
                .next()
                .map(|occurrence| occurrence.with_timezone(&Utc)))
        }
        JobSchedule::RecurringInterval { every_seconds } => {
            let seconds = interval_seconds(*every_seconds)?;
            Ok(Some(after + Duration::seconds(seconds)))
        }
        JobSchedule::OneShot { fire_at } => Ok((*fire_at > after).then_some(*fire_at)),
    }
}

fn parse_cron(expression: &str) -> Result<cron::Schedule, OccurrenceError> {
    format!("0 {expression} *")
        .parse()
        .map_err(|source| OccurrenceError::InvalidCron {
            expression: expression.to_string(),
            source,
        })
}

fn interval_seconds(every_seconds: u64) -> Result<i64, OccurrenceError> {
    if every_seconds == 0 {
        return Err(OccurrenceError::InvalidInterval);
    }
    Ok(i64::try_from(every_seconds).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 11, 22, 0, 0).unwrap() + Duration::seconds(seconds)
    }

    #[test]
    fn cron_returns_latest_due_occurrence() {
        let schedule = JobSchedule::RecurringCron {
            expr: "* * * * *".to_string(),
        };
        assert_eq!(
            latest_due(&schedule, at(0), at(150)).unwrap(),
            Some(at(120))
        );
    }

    #[test]
    fn interval_returns_latest_due_occurrence() {
        let schedule = JobSchedule::RecurringInterval { every_seconds: 10 };
        assert_eq!(latest_due(&schedule, at(0), at(35)).unwrap(), Some(at(30)));
    }

    #[test]
    fn interval_returns_every_due_occurrence() {
        let schedule = JobSchedule::RecurringInterval { every_seconds: 10 };
        assert_eq!(
            due_after(&schedule, at(10), at(35)).unwrap(),
            vec![at(20), at(30)]
        );
    }

    #[test]
    fn interval_returns_next_occurrence() {
        let schedule = JobSchedule::RecurringInterval { every_seconds: 10 };
        assert_eq!(next_after(&schedule, at(10)).unwrap(), Some(at(20)));
    }

    #[test]
    fn one_shot_is_due_once_inside_the_range() {
        let schedule = JobSchedule::OneShot { fire_at: at(10) };
        assert_eq!(latest_due(&schedule, at(0), at(10)).unwrap(), Some(at(10)));
        assert_eq!(latest_due(&schedule, at(10), at(20)).unwrap(), None);
    }

    #[test]
    fn invalid_interval_is_rejected() {
        let schedule = JobSchedule::RecurringInterval { every_seconds: 0 };
        assert!(matches!(
            next_after(&schedule, at(0)),
            Err(OccurrenceError::InvalidInterval)
        ));
    }
}
