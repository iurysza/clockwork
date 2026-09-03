mod helpers;

use std::fs;

use chrono::{Duration, Utc};
use helpers::TestEnv;
use serde_json::Value;

fn json(env: &TestEnv, args: &[&str]) -> Value {
    let output = env.cmd().args(args).output().expect("run clockwork");
    assert!(
        output.status.success(),
        "command failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("one JSON document")
}

fn apply(env: &TestEnv, base: &[&str]) -> Value {
    let mut preview_args = base.to_vec();
    preview_args.extend(["--dry-run", "--json"]);
    let preview = json(env, &preview_args);
    let mut args = base.to_vec();
    args.extend([
        "--yes",
        "--if-revision",
        preview["revision"].as_str().unwrap(),
        "--json",
    ]);
    json(env, &args)
}

fn create(env: &TestEnv, name: &str, schedule: &str) {
    apply(
        env,
        &[
            "job",
            "create",
            name,
            "--schedule",
            schedule,
            "--command",
            "echo before",
        ],
    );
}

#[test]
fn update_preserves_activation_after_validating_the_complete_definition() {
    let env = TestEnv::new();
    create(&env, "editable", "every 1h");
    apply(&env, &["job", "enable", "editable"]);

    let result = apply(
        &env,
        &["job", "update", "editable", "--command", "echo after"],
    );
    assert_eq!(result["changed"], true);
    assert_eq!(result["state"]["type"], "scheduled");
    assert!(result["state"]["next_run"].as_str().is_some());

    let source = fs::read_to_string(env.jobs_dir().join("editable/clockwork.yaml")).unwrap();
    assert!(source.contains("echo after"));
    assert!(!source.contains("paused"));
    let status = json(&env, &["job", "status", "editable", "--json"]);
    assert_eq!(status["state"]["type"], "scheduled");
}

#[test]
fn updating_a_completed_one_time_schedule_creates_a_disabled_generation() {
    let env = TestEnv::new();
    let initial_schedule = (Utc::now() + Duration::hours(1)).to_rfc3339();
    create(&env, "once", &initial_schedule);
    apply(&env, &["job", "enable", "once"]);

    let state_path = env.home().join("jobs.json");
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    state["jobs"]["once"]["status"] = Value::String("completed".to_string());
    state["jobs"]["once"]["completed_at"] = Value::String(Utc::now().to_rfc3339());
    fs::write(&state_path, serde_json::to_string(&state).unwrap()).unwrap();

    let next_schedule = (Utc::now() + Duration::hours(2)).to_rfc3339().to_string();
    let result = apply(
        &env,
        &["job", "update", "once", "--schedule", &next_schedule],
    );
    assert_eq!(result["state"]["type"], "disabled");
    assert_eq!(result["state"]["runtime_generation"], 1);

    let status = json(&env, &["job", "status", "once", "--json"]);
    assert_eq!(status["activation"], "disabled");
    assert_eq!(status["state"]["runtime_generation"], 1);
}

#[test]
fn update_rejects_an_in_flight_run_before_writing_source_or_runtime() {
    let env = TestEnv::new();
    create(&env, "busy", "every 1h");
    apply(&env, &["job", "enable", "busy"]);

    let state_path = env.home().join("jobs.json");
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    state["jobs"]["busy"]["in_flight"] = serde_json::json!({
        "run_id": "run_busy",
        "scheduled_for": Utc::now().to_rfc3339(),
        "claimed_at": Utc::now().to_rfc3339(),
    });
    fs::write(&state_path, serde_json::to_string(&state).unwrap()).unwrap();
    let source_before = fs::read_to_string(env.jobs_dir().join("busy/clockwork.yaml")).unwrap();
    let runtime_before = fs::read_to_string(&state_path).unwrap();

    let output = env
        .cmd()
        .args([
            "job",
            "update",
            "busy",
            "--command",
            "echo forbidden",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_RUN_IN_FLIGHT");
    assert_eq!(
        source_before,
        fs::read_to_string(env.jobs_dir().join("busy/clockwork.yaml")).unwrap()
    );
    assert_eq!(runtime_before, fs::read_to_string(&state_path).unwrap());
}
