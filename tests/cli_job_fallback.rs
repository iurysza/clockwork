//! Fallback coverage for managed jobs: the retained public path is the
//! global `on_failure` config; per-job failure commands left with `add`.
mod helpers;

use std::fs;
use std::thread;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use helpers::TestEnv;
use predicates::prelude::*;
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

fn history_json(env: &TestEnv, job: &str) -> Vec<Value> {
    json(env, &["job", "history", job, "--limit", "200", "--json"])["runs"]
        .as_array()
        .unwrap()
        .clone()
}

fn wait_for_history(env: &TestEnv, job: &str, count: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if history_json(env, job).len() >= count {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "expected at least {} history records, got {}",
        count,
        history_json(env, job).len()
    );
}

fn set_global_fallback(env: &TestEnv, marker_command: &str) {
    env.cmd()
        .args(["config", "on_failure", marker_command])
        .assert()
        .success();
    env.cmd()
        .args(["config", "on_failure_shell", "true"])
        .assert()
        .success();
}

#[test]
fn failed_job_triggers_global_fallback_and_records_history() {
    let env = TestEnv::new();
    let marker = env.home().join("fallback-ran.txt");
    set_global_fallback(
        &env,
        &format!("echo fallback-executed > {}", marker.display()),
    );
    let schedule = (Utc::now() + ChronoDuration::seconds(1)).to_rfc3339();
    create_and_enable(&env, "fail-job", &schedule, "false");

    thread::sleep(Duration::from_millis(1200));
    env.cmd().args(["_internal", "dispatch"]).assert().success();

    wait_for_history(&env, "fail-job", 2);

    let history = history_json(&env, "fail-job");
    let main_run = history
        .iter()
        .find(|record| record["trigger"] == "scheduled")
        .expect("main run in history");
    assert_eq!(main_run["status"], "failed");

    let fallback_run = history
        .iter()
        .find(|record| record["trigger"] == "fallback")
        .expect("fallback run in history");
    assert_eq!(fallback_run["status"], "success");
    assert_eq!(
        fallback_run["failed_run_id"].as_str().unwrap(),
        main_run["run_id"].as_str().unwrap()
    );
    assert!(marker.exists(), "fallback should have created the marker");
}

#[test]
fn successful_job_does_not_trigger_fallback() {
    let env = TestEnv::new();
    let marker = env.home().join("should-not-exist.txt");
    set_global_fallback(&env, &format!("touch {}", marker.display()));
    let schedule = (Utc::now() + ChronoDuration::seconds(1)).to_rfc3339();
    create_and_enable(&env, "success-job", &schedule, "true");

    thread::sleep(Duration::from_millis(1200));
    env.cmd().args(["_internal", "dispatch"]).assert().success();

    wait_for_history(&env, "success-job", 1);
    thread::sleep(Duration::from_millis(500));

    let history = history_json(&env, "success-job");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["status"], "success");
    assert!(!marker.exists(), "fallback must not run for success");
}

#[test]
fn failures_log_written_on_failure() {
    let env = TestEnv::new();
    let schedule = (Utc::now() + ChronoDuration::seconds(1)).to_rfc3339();
    create_and_enable(&env, "log-fail", &schedule, "false");

    thread::sleep(Duration::from_millis(1200));
    env.cmd().args(["_internal", "dispatch"]).assert().success();
    wait_for_history(&env, "log-fail", 1);
    thread::sleep(Duration::from_millis(300));

    let failures_log = env.home().join("failures.log");
    let content = fs::read_to_string(&failures_log).expect("failures.log should exist");
    assert!(content.contains("FAILED job=\"log-fail\""));
    assert!(content.contains("status=failed"));
}

#[test]
fn trigger_fires_global_fallback_on_failure() {
    let env = TestEnv::new();
    let marker = env.home().join("manual-fallback.txt");
    set_global_fallback(&env, &format!("touch {}", marker.display()));
    create_and_enable(&env, "manual-fail", "every 1h", "false");

    apply(&env, &["job", "trigger", "manual-fail"]);

    thread::sleep(Duration::from_secs(2));
    let history = history_json(&env, "manual-fail");
    assert!(
        history.iter().any(|record| record["trigger"] == "fallback"),
        "trigger should fire the global fallback"
    );
    assert!(marker.exists(), "fallback should have created the marker");
}

#[test]
fn backward_compat_jobs_without_on_failure_load_fine() {
    let env = TestEnv::new();
    create_and_enable(&env, "compat-job", "every 1h", "echo test");

    // Simulate pre-on_failure state: strip the fields from jobs.json.
    let jobs_path = env.home().join("jobs.json");
    let content = fs::read_to_string(&jobs_path).expect("read jobs.json");
    let mut jobs: Value = serde_json::from_str(&content).expect("parse jobs.json");
    for (_id, job) in jobs["jobs"].as_object_mut().expect("jobs object") {
        let obj = job.as_object_mut().expect("job object");
        obj.remove("on_failure");
        obj.remove("on_failure_shell");
    }
    fs::write(
        &jobs_path,
        serde_json::to_string_pretty(&jobs).expect("serialize"),
    )
    .expect("write");

    // The managed view still loads and resolves the job.
    let status = json(&env, &["job", "status", "compat-job", "--json"]);
    assert_eq!(status["job"], "compat-job");
}

#[test]
fn config_on_failure_set_and_read() {
    let env = TestEnv::new();

    env.cmd()
        .args(["config", "on_failure", "echo global failure"])
        .assert()
        .success();

    env.cmd()
        .args(["config", "on_failure"])
        .assert()
        .success()
        .stdout(predicate::str::contains("echo global failure"));

    env.cmd()
        .args(["config", "on_failure", ""])
        .assert()
        .success();

    env.cmd()
        .args(["config", "on_failure"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(none)"));
}
