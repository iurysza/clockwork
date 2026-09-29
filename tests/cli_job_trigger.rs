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
    assert_eq!(
        record["log_path"],
        format!("logs/now/{}.log", record["run_id"].as_str().unwrap())
    );
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
fn a_stale_manual_worker_cannot_repeat_a_completed_action() {
    let env = TestEnv::new();
    create_and_enable_with_command(&env, "once-only", "every 1h", "echo completed");
    apply(&env, &["job", "trigger", "once-only"]);

    let history = json(&env, &["job", "history", "once-only", "--json"]);
    let record = &history["runs"][0];
    env.cmd()
        .args([
            "_internal",
            "execute",
            "once-only",
            "--scheduled-for",
            record["scheduled_for"].as_str().unwrap(),
            "--trigger",
            "manual",
            "--run-id",
            record["run_id"].as_str().unwrap(),
        ])
        .assert()
        .failure();
    let after = json(&env, &["job", "history", "once-only", "--json"]);
    assert_eq!(after["runs"].as_array().unwrap().len(), 1);
    let state: Value =
        serde_json::from_str(&fs::read_to_string(env.home().join("jobs.json")).unwrap()).unwrap();
    assert_eq!(state["jobs"]["once-only"]["run_count"], 1);
}

#[test]
fn recovery_keeps_manual_trigger_and_existing_log_path() {
    let env = TestEnv::new();
    create_and_enable(&env, "recovered", "every 1h");
    let state_path = env.home().join("jobs.json");
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    let claim_time = Utc::now() - Duration::minutes(1);
    state["jobs"]["recovered"]["in_flight"] = serde_json::json!({
        "run_id": "abandoned",
        "scheduled_for": claim_time.to_rfc3339(),
        "claimed_at": claim_time.to_rfc3339(),
        "trigger": "manual",
    });
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    let log_dir = env.home().join("logs/recovered");
    fs::create_dir_all(&log_dir).unwrap();
    fs::write(log_dir.join("abandoned.log"), "inert marker\n").unwrap();

    env.cmd().args(["_internal", "dispatch"]).assert().success();
    let history = json(&env, &["job", "history", "recovered", "--json"]);
    let record = &history["runs"][0];
    assert_eq!(record["run_id"], "abandoned");
    assert_eq!(record["trigger"], "manual");
    assert_eq!(record["status"], "internal_error");
    assert_eq!(record["log_path"], "logs/recovered/abandoned.log");
    let state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    assert!(state["jobs"]["recovered"]["in_flight"].is_null());
    assert_eq!(state["jobs"]["recovered"]["run_count"], 0);
}

#[cfg(unix)]
#[test]
fn interrupted_trigger_leaves_a_worker_to_record_completion() {
    let env = TestEnv::new();
    apply(
        &env,
        &[
            "job",
            "create",
            "interrupted",
            "--schedule",
            "every 1h",
            "--command",
            "echo begun; sleep 3; echo completed",
            "--shell",
        ],
    );
    apply(&env, &["job", "enable", "interrupted"]);
    // The shell command is inert and exists only in the temporary test state.
    let preview = json(
        &env,
        &["job", "trigger", "interrupted", "--dry-run", "--json"],
    );
    let mut cli = std::process::Command::new(assert_cmd::cargo::cargo_bin!("clockwork"))
        .env("CLOCKWORK_HOME", env.home())
        .env("CLOCKWORK_JOBS_ROOT", env.jobs_dir())
        .env("CLOCKWORK_BACKEND", "none")
        .args([
            "job",
            "trigger",
            "interrupted",
            "--yes",
            "--if-revision",
            preview["revision"].as_str().unwrap(),
            "--json",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start test-owned CLI");
    let deadline = Instant::now() + StdDuration::from_secs(5);
    let run_id = loop {
        let state: Value =
            serde_json::from_str(&fs::read_to_string(env.home().join("jobs.json")).unwrap())
                .unwrap();
        if let Some(id) = state["jobs"]["interrupted"]["in_flight"]["run_id"].as_str() {
            let log = env.home().join(format!("logs/interrupted/{id}.log"));
            if fs::read_to_string(log).is_ok_and(|contents| contents.contains("begun")) {
                break id.to_string();
            }
        }
        assert!(Instant::now() < deadline, "test worker did not start");
        thread::sleep(StdDuration::from_millis(20));
    };

    let pid = i32::try_from(cli.id()).unwrap();
    assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
    assert!(!cli.wait().unwrap().success());
    let state_path = env.home().join("jobs.json");
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    assert_eq!(state["jobs"]["interrupted"]["in_flight"]["run_id"], run_id);
    state["jobs"]["interrupted"]["in_flight"]["claimed_at"] =
        Value::String((Utc::now() - Duration::minutes(1)).to_rfc3339());
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    env.cmd().args(["_internal", "dispatch"]).assert().success();
    let state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    assert_eq!(state["jobs"]["interrupted"]["in_flight"]["run_id"], run_id);
    assert!(!env.home().join("run-history.jsonl").exists());

    let deadline = Instant::now() + StdDuration::from_secs(8);
    loop {
        let history = json(&env, &["job", "history", "interrupted", "--json"]);
        if let Some(record) = history["runs"].as_array().unwrap().first() {
            assert_eq!(record["run_id"], run_id);
            assert_eq!(record["trigger"], "manual");
            assert_eq!(record["status"], "success");
            assert_eq!(record["log_path"], format!("logs/interrupted/{run_id}.log"));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not record completion"
        );
        thread::sleep(StdDuration::from_millis(25));
    }
    let state: Value =
        serde_json::from_str(&fs::read_to_string(env.home().join("jobs.json")).unwrap()).unwrap();
    assert!(state["jobs"]["interrupted"]["in_flight"].is_null());
    assert_eq!(state["jobs"]["interrupted"]["run_count"], 1);
    assert!(
        fs::read_to_string(env.home().join(format!("logs/interrupted/{run_id}.log")))
            .unwrap()
            .contains("completed")
    );
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
