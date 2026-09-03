mod helpers;

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Utc};
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

fn create_and_enable(env: &TestEnv, name: &str, schedule: &str, command: &str) {
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

fn wait_for_run(env: &TestEnv, name: &str) -> Value {
    // The dispatcher detaches the executor. Three seconds accommodates the
    // one-second test schedule and process startup on a loaded CI worker.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let history = json(env, &["job", "history", name, "--json"]);
        if !history["runs"].as_array().unwrap().is_empty() {
            return history;
        }
        assert!(Instant::now() < deadline, "dispatcher did not record a run");
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn enabling_a_recurring_job_does_not_run_disabled_time_as_backlog() {
    let env = TestEnv::new();
    apply(
        &env,
        &[
            "job",
            "create",
            "fresh-window",
            "--schedule",
            "every 1s",
            "--command",
            "true",
        ],
    );
    thread::sleep(Duration::from_millis(1200));
    apply(&env, &["job", "enable", "fresh-window"]);

    env.cmd().args(["_internal", "dispatch"]).assert().success();
    let state: Value =
        serde_json::from_str(&fs::read_to_string(env.home().join("jobs.json")).unwrap()).unwrap();
    assert_eq!(state["jobs"]["fresh-window"]["run_count"], 0);
    assert!(state["jobs"]["fresh-window"]["in_flight"].is_null());
}

#[test]
fn completed_managed_jobs_are_not_archived_by_legacy_cleanup() {
    let env = TestEnv::new();
    let schedule = (Utc::now() + ChronoDuration::hours(1)).to_rfc3339();
    create_and_enable(&env, "one-shot", &schedule, "true");
    apply(&env, &["job", "trigger", "one-shot"]);
    let validation = json(&env, &["job", "validate", "one-shot", "--json"]);
    assert_eq!(validation["ok"], true);

    env.cmd()
        .args(["config", "archive_after_hours", "1"])
        .assert()
        .success();
    let state_path = env.home().join("jobs.json");
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    let old = (Utc::now() - ChronoDuration::hours(2)).to_rfc3339();
    state["jobs"]["one-shot"]["completed_at"] = Value::String(old.clone());
    state["jobs"]["one-shot"]["updated_at"] = Value::String(old);
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();

    env.cmd().args(["_internal", "dispatch"]).assert().success();
    let status = json(&env, &["job", "status", "one-shot", "--json"]);
    assert_eq!(status["state"]["type"], "completed");
}

#[test]
fn source_drift_blocks_only_the_changed_job() {
    let env = TestEnv::new();
    create_and_enable(&env, "changed", "every 1s", "echo original");
    create_and_enable(&env, "healthy", "every 1s", "echo healthy-output");

    let source_path = env.jobs_dir().join("changed/clockwork.yaml");
    let source = fs::read_to_string(&source_path).unwrap();
    fs::write(
        &source_path,
        source.replace("echo original", "echo unreviewed"),
    )
    .unwrap();

    thread::sleep(Duration::from_millis(1200));
    env.cmd().args(["_internal", "dispatch"]).assert().success();
    let history = wait_for_run(&env, "healthy");
    assert_eq!(history["runs"][0]["status"], "success");

    let state: Value =
        serde_json::from_str(&fs::read_to_string(env.home().join("jobs.json")).unwrap()).unwrap();
    assert_eq!(state["jobs"]["changed"]["run_count"], 0);
    assert!(state["jobs"]["changed"]["in_flight"].is_null());
}

#[test]
fn internal_dispatch_runs_an_enabled_due_managed_job_and_records_a_log() {
    let env = TestEnv::new();
    create_and_enable(&env, "scheduled", "every 1s", "echo dispatched-output");

    // `every 1s` becomes due after one second; 1.2s leaves 200ms margin.
    thread::sleep(Duration::from_millis(1200));
    env.cmd().args(["_internal", "dispatch"]).assert().success();

    let history = wait_for_run(&env, "scheduled");
    assert_eq!(history["runs"][0]["trigger"], "scheduled");
    assert_eq!(history["runs"][0]["status"], "success");

    let log = json(&env, &["job", "logs", "scheduled", "--json"]);
    assert!(log["log"].as_str().unwrap().contains("dispatched-output"));

    let state: Value =
        serde_json::from_str(&fs::read_to_string(env.home().join("jobs.json")).unwrap()).unwrap();
    assert!(state["jobs"]["scheduled"]["in_flight"].is_null());
    assert_eq!(state["jobs"]["scheduled"]["run_count"], 1);
}
