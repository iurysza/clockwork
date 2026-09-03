mod helpers;

use std::fs;
use std::thread;
use std::time::{Duration as StdDuration, Instant};

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

fn create_and_enable(env: &TestEnv, name: &str, schedule: &str) {
    create_and_enable_with_command(env, name, schedule, "true");
}

fn create_and_enable_with_command(env: &TestEnv, name: &str, schedule: &str, command: &str) {
    apply(
        env,
        &[
            "job",
            "create",
            name,
            "--schedule",
            schedule,
            "--command",
            command,
        ],
    );
    apply(env, &["job", "enable", name]);
}

fn wait_for_claim(env: &TestEnv, name: &str) {
    // The action sleeps for two seconds. A one-second observation window
    // leaves a full second of execution after the claim becomes visible.
    let deadline = Instant::now() + StdDuration::from_secs(1);
    loop {
        let claimed = fs::read_to_string(env.home().join("jobs.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|state| state["jobs"][name]["in_flight"].as_object().cloned())
            .is_some();
        if claimed {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "trigger did not create an in-flight claim"
        );
        thread::sleep(StdDuration::from_millis(10));
    }
}

#[test]
fn trigger_previews_without_running_then_records_the_manual_execution() {
    let env = TestEnv::new();
    create_and_enable(&env, "now", "every 1h");

    let preview = json(&env, &["job", "trigger", "now", "--dry-run", "--json"]);
    assert_eq!(preview["changed"], false);
    assert_eq!(preview["changes"], serde_json::json!(["trigger_run"]));
    assert_eq!(preview["external_effect"]["type"], "immediate_trigger");
    assert!(!env.home().join("run-history.jsonl").exists());
    let before = json(&env, &["job", "status", "now", "--json"]);

    let result = apply(&env, &["job", "trigger", "now"]);
    assert_eq!(result["changed"], true);
    assert_eq!(result["state"]["type"], "scheduled");
    assert_eq!(result["state"]["next_run"], before["state"]["next_run"]);

    let history = fs::read_to_string(env.home().join("run-history.jsonl")).unwrap();
    let record: Value = serde_json::from_str(history.lines().next().unwrap()).unwrap();
    assert_eq!(record["job_id"], "now");
    assert_eq!(record["trigger"], "manual");
    assert_eq!(record["status"], "success");
}

#[test]
fn trigger_completes_a_one_time_job_through_the_normal_executor_path() {
    let env = TestEnv::new();
    let schedule = (Utc::now() + Duration::hours(1)).to_rfc3339();
    create_and_enable(&env, "once", &schedule);

    let result = apply(&env, &["job", "trigger", "once"]);
    assert_eq!(result["changed"], true);
    assert_eq!(result["state"]["type"], "completed");

    let status = json(&env, &["job", "status", "once", "--json"]);
    assert_eq!(status["activation"], "disabled");
    assert_eq!(status["state"]["type"], "completed");
}

#[test]
fn trigger_rejects_disabled_and_in_flight_jobs_without_executing() {
    let env = TestEnv::new();
    apply(
        &env,
        &[
            "job",
            "create",
            "idle",
            "--schedule",
            "every 1h",
            "--command",
            "true",
        ],
    );

    let disabled = env
        .cmd()
        .args(["job", "trigger", "idle", "--dry-run", "--json"])
        .output()
        .unwrap();
    assert!(!disabled.status.success());
    let error: Value = serde_json::from_slice(&disabled.stdout).unwrap();
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_ILLEGAL_TRANSITION");

    apply(&env, &["job", "enable", "idle"]);
    let state_path = env.home().join("jobs.json");
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    state["jobs"]["idle"]["in_flight"] = serde_json::json!({
        "run_id": "run_busy",
        "scheduled_for": Utc::now().to_rfc3339(),
        "claimed_at": Utc::now().to_rfc3339(),
    });
    fs::write(&state_path, serde_json::to_string(&state).unwrap()).unwrap();

    let busy = env
        .cmd()
        .args(["job", "trigger", "idle", "--dry-run", "--json"])
        .output()
        .unwrap();
    assert!(!busy.status.success());
    let error: Value = serde_json::from_slice(&busy.stdout).unwrap();
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_RUN_IN_FLIGHT");
    assert!(!env.home().join("run-history.jsonl").exists());
}

#[test]
fn update_and_delete_reject_while_a_triggered_action_is_running() {
    let env = TestEnv::new();
    create_and_enable_with_command(&env, "busy", "every 1h", "sleep 2");

    let preview = json(&env, &["job", "trigger", "busy", "--dry-run", "--json"]);
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin!("clockwork"))
        .env("CLOCKWORK_HOME", env.home())
        .env("CLOCKWORK_JOBS_ROOT", env.jobs_dir())
        .env("CLOCKWORK_BACKEND", "none")
        .args([
            "job",
            "trigger",
            "busy",
            "--yes",
            "--if-revision",
            preview["revision"].as_str().unwrap(),
            "--json",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start trigger");
    wait_for_claim(&env, "busy");

    for args in [
        &[
            "job",
            "update",
            "busy",
            "--command",
            "echo forbidden",
            "--dry-run",
            "--json",
        ][..],
        &["job", "delete", "busy", "--dry-run", "--json"][..],
    ] {
        let output = env.cmd().args(args).output().unwrap();
        assert!(!output.status.success());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["changed"], false);
        assert_eq!(error["error"]["code"], "CW_RUN_IN_FLIGHT");
    }

    assert!(child.wait().unwrap().success());
    let status = json(&env, &["job", "status", "busy", "--json"]);
    assert_eq!(status["state"]["type"], "scheduled");
}
