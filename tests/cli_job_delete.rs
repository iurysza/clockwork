mod helpers;

use std::fs;

use chrono::Utc;
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

fn create(env: &TestEnv, name: &str) {
    apply(
        env,
        &[
            "job",
            "create",
            name,
            "--schedule",
            "every 1h",
            "--command",
            "echo managed",
        ],
    );
}

#[test]
fn delete_removes_the_disabled_runtime_and_managed_source() {
    let env = TestEnv::new();
    create(&env, "obsolete");

    let preview = json(&env, &["job", "delete", "obsolete", "--dry-run", "--json"]);
    assert_eq!(preview["changed"], false);
    assert_eq!(
        preview["changes"],
        serde_json::json!(["remove_runtime", "remove_source"])
    );

    let result = apply(&env, &["job", "delete", "obsolete"]);
    assert_eq!(result["changed"], true);
    assert!(result["state"].is_null());
    assert!(!env.jobs_dir().join("obsolete/clockwork.yaml").exists());

    let runtime: Value =
        serde_json::from_str(&fs::read_to_string(env.home().join("jobs.json")).unwrap()).unwrap();
    assert!(runtime["jobs"].get("obsolete").is_none());
}

#[test]
fn delete_refuses_a_source_directory_with_unknown_user_files() {
    let env = TestEnv::new();
    create(&env, "guarded");
    let note = env.jobs_dir().join("guarded/notes.txt");
    fs::write(&note, "do not delete\n").unwrap();

    let output = env
        .cmd()
        .args(["job", "delete", "guarded", "--dry-run", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_SOURCE_FAILURE");
    assert_eq!(fs::read_to_string(note).unwrap(), "do not delete\n");
    assert!(env.home().join("jobs.json").exists());
}

#[test]
fn delete_rejects_a_stale_preview_before_mutating() {
    let env = TestEnv::new();
    create(&env, "current");

    let preview = json(&env, &["job", "delete", "current", "--dry-run", "--json"]);
    apply(&env, &["job", "enable", "current"]);

    let output = env
        .cmd()
        .args([
            "job",
            "delete",
            "current",
            "--yes",
            "--if-revision",
            preview["revision"].as_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_REVISION_CONFLICT");
    assert!(env.jobs_dir().join("current/clockwork.yaml").exists());
}

#[test]
fn delete_rejects_an_in_flight_job_without_removing_its_source() {
    let env = TestEnv::new();
    create(&env, "busy");
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

    let output = env
        .cmd()
        .args(["job", "delete", "busy", "--dry-run", "--json"])
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
}
