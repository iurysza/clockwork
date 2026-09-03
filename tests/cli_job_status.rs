mod helpers;

use std::fs;

use helpers::TestEnv;
use serde_json::Value;

fn command_json(env: &TestEnv, args: &[&str]) -> Value {
    let output = env.cmd().args(args).output().expect("run clockwork");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("one JSON document")
}

fn create(env: &TestEnv, name: &str) {
    let preview = command_json(
        env,
        &[
            "job",
            "create",
            name,
            "--schedule",
            "every 1h",
            "--command",
            "true",
            "--dry-run",
            "--json",
        ],
    );
    command_json(
        env,
        &[
            "job",
            "create",
            name,
            "--schedule",
            "every 1h",
            "--command",
            "true",
            "--yes",
            "--if-revision",
            preview["revision"].as_str().unwrap(),
            "--json",
        ],
    );
}

#[test]
fn status_resolves_source_runtime_activation_generation_and_revision() {
    let env = TestEnv::new();
    create(&env, "inspect-me");

    let status = command_json(&env, &["job", "status", "inspect-me", "--json"]);
    assert_eq!(status["ok"], true);
    assert_eq!(status["job"], "inspect-me");
    assert_eq!(status["state"]["type"], "disabled");
    assert_eq!(status["state"]["runtime_generation"], 0);
    assert_eq!(status["activation"], "disabled");
    assert_eq!(status["schedule"], "every 1h");
    assert_eq!(status["action"], "command");
    assert!(
        status["revision"]
            .as_str()
            .is_some_and(|value| value.starts_with("rev_"))
    );

    let list = command_json(&env, &["job", "status", "--json"]);
    let jobs = list["jobs"].as_array().expect("status list");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["job"], "inspect-me");
}

#[test]
fn status_reports_manually_changed_runtime_definition_as_an_integrity_violation() {
    let env = TestEnv::new();
    create(&env, "changed-runtime");
    let state_path = env.home().join("jobs.json");
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    state["jobs"]["changed-runtime"]["action"]["command"] = Value::String("false".to_string());
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();

    let output = env
        .cmd()
        .args(["job", "status", "changed-runtime", "--json"])
        .output()
        .expect("run clockwork");
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).expect("one JSON error");
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_INTEGRITY_VIOLATION");

    state["jobs"]["changed-runtime"]["action"]["command"] = Value::String("true".to_string());
    state["jobs"]["changed-runtime"]["on_failure"] =
        Value::String("touch /tmp/unreviewed".to_string());
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    let output = env
        .cmd()
        .args(["job", "status", "changed-runtime", "--json"])
        .output()
        .expect("run clockwork");
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).expect("one JSON error");
    assert_eq!(error["error"]["code"], "CW_INTEGRITY_VIOLATION");
}

#[test]
fn status_reports_runtime_without_source_as_an_integrity_violation() {
    let env = TestEnv::new();
    create(&env, "orphaned");
    fs::remove_dir_all(env.jobs_dir().join("orphaned")).unwrap();

    let output = env
        .cmd()
        .args(["job", "status", "orphaned", "--json"])
        .output()
        .expect("run clockwork");
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).expect("one JSON error");
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_INTEGRITY_VIOLATION");
}

#[test]
fn status_of_a_missing_job_is_a_typed_pre_mutation_failure() {
    let env = TestEnv::new();
    let output = env
        .cmd()
        .args(["job", "status", "missing", "--json"])
        .output()
        .expect("run clockwork");
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).expect("one JSON error");
    assert_eq!(error["ok"], false);
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_JOB_NOT_FOUND");
}
