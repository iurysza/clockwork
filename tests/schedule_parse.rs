//! Integration coverage for the schedule grammar exposed by `clockwork job create`.
mod helpers;

use helpers::TestEnv;
use serde_json::Value;

fn preview(env: &TestEnv, name: &str, schedule: &str) -> std::process::Output {
    env.cmd()
        .args([
            "job",
            "create",
            name,
            "--schedule",
            schedule,
            "--command",
            "echo test",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("run job create preview")
}

fn valid_preview(env: &TestEnv, name: &str, schedule: &str) -> Value {
    let output = preview(env, name, schedule);
    assert!(
        output.status.success(),
        "preview failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn invalid_preview(env: &TestEnv, schedule: &str) -> Value {
    let output = preview(env, "invalid", schedule);
    assert!(
        !output.status.success(),
        "invalid schedule unexpectedly passed"
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn schedule_every_zero_rejected() {
    let error = invalid_preview(&TestEnv::new(), "every 0h");
    assert_eq!(error["error"]["code"], "CW_INVALID_INPUT");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("greater than zero")
    );
}

#[test]
fn schedule_every_minutes_max() {
    valid_preview(&TestEnv::new(), "max-min", "every 59m");
    invalid_preview(&TestEnv::new(), "every 60m");
}

#[test]
fn schedule_relative_and_bare_durations_create_one_shot_previews() {
    for (name, schedule) in [("relative", "in 4h"), ("seconds", "10s"), ("minutes", "5m")] {
        let plan = valid_preview(&TestEnv::new(), name, schedule);
        assert_eq!(plan["expected_state"]["type"], "disabled");
        assert_eq!(plan["external_effect"]["type"], "none");
    }
}

#[test]
fn relative_one_shot_preview_provides_the_absolute_apply_value() {
    let env = TestEnv::new();
    let plan = valid_preview(&env, "relative-apply", "in 4h");
    let schedule = plan["schedule"].as_str().expect("normalized schedule");
    assert!(chrono::DateTime::parse_from_rfc3339(schedule).is_ok());

    let rejected = env
        .cmd()
        .args([
            "job",
            "create",
            "relative-apply",
            "--schedule",
            "in 4h",
            "--command",
            "echo test",
            "--yes",
            "--if-revision",
            plan["revision"].as_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let error: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(error["error"]["code"], "CW_INVALID_INPUT");

    let applied = env
        .cmd()
        .args([
            "job",
            "create",
            "relative-apply",
            "--schedule",
            schedule,
            "--command",
            "echo test",
            "--yes",
            "--if-revision",
            plan["revision"].as_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(applied.status.success());
}

#[test]
fn schedule_iso_datetime_accepts_future_and_rejects_past() {
    valid_preview(&TestEnv::new(), "future", "2099-12-31T23:59:59Z");
    let error = invalid_preview(&TestEnv::new(), "2020-01-01T00:00:00Z");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("in the past")
    );
}

#[test]
fn schedule_invalid_string_and_cron_are_checked_by_the_public_parser() {
    let invalid = invalid_preview(&TestEnv::new(), "tomorrow");
    assert!(
        invalid["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Could not parse schedule")
    );
    valid_preview(&TestEnv::new(), "weekday", "0 9 * * 1-5");
    invalid_preview(&TestEnv::new(), "99 99 99 99 99");
}

#[test]
fn schedule_every_seconds_recurs_and_rejects_zero() {
    valid_preview(&TestEnv::new(), "ticker", "every 30s");
    invalid_preview(&TestEnv::new(), "every 0s");
}
