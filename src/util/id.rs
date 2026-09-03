use chrono::Utc;

const JOB_ID_ALPHABET: &str = "0123456789abcdefghijklmnopqrstuvwxyz";
const RUN_ID_PREFIX: &str = "r_";

/// Generate a run ID with timestamp prefix for ordering.
pub fn new_run_id() -> String {
    let ts = Utc::now().format("%Y%m%d%H%M%S");
    let suffix = nanoid::nanoid!(6, &JOB_ID_ALPHABET.chars().collect::<Vec<_>>());
    format!("{RUN_ID_PREFIX}{ts}_{suffix}")
}
