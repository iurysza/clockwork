mod helpers;

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

#[test]
fn list_history_and_logs_resolve_the_same_managed_job_identity() {
    let env = TestEnv::new();
    apply(
        &env,
        &[
            "job",
            "create",
            "observed",
            "--schedule",
            "every 1h",
            "--command",
            "echo visible-output",
        ],
    );
    apply(&env, &["job", "enable", "observed"]);
    apply(&env, &["job", "trigger", "observed"]);

    let list = json(&env, &["job", "list", "--json"]);
    assert_eq!(list["ok"], true);
    assert_eq!(list["jobs"].as_array().unwrap().len(), 1);
    assert_eq!(list["jobs"][0]["job"], "observed");
    assert_eq!(list["jobs"][0]["state"]["type"], "scheduled");

    let history = json(&env, &["job", "history", "observed", "--json"]);
    assert_eq!(history["ok"], true);
    assert_eq!(history["job"], "observed");
    assert_eq!(history["runs"].as_array().unwrap().len(), 1);
    let run_id = history["runs"][0]["run_id"].as_str().unwrap();
    assert_eq!(history["runs"][0]["trigger"], "manual");

    let latest_log = json(&env, &["job", "logs", "observed", "--json"]);
    assert_eq!(latest_log["job"], "observed");
    assert!(
        latest_log["log"]
            .as_str()
            .unwrap()
            .contains("visible-output")
    );

    let selected_log = json(
        &env,
        &["job", "logs", "observed", "--run", run_id, "--json"],
    );
    assert_eq!(selected_log["run"], run_id);
    assert!(
        selected_log["log"]
            .as_str()
            .unwrap()
            .contains("visible-output")
    );
}

#[test]
fn list_hides_action_secrets_and_reports_managed_tags() {
    let env = TestEnv::new();
    apply(
        &env,
        &[
            "job",
            "create",
            "ops",
            "--schedule",
            "every 1h",
            "--command",
            "echo super-secret-command",
            "--tag",
            "ops",
        ],
    );
    apply(
        &env,
        &[
            "job",
            "create",
            "other",
            "--schedule",
            "every 1h",
            "--command",
            "echo ordinary",
        ],
    );

    let output = env.cmd().args(["job", "list", "--json"]).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("super-secret-command"));
    let list: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(list["jobs"].as_array().unwrap().len(), 2);
    let ops = list["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["job"] == "ops")
        .unwrap();
    assert_eq!(ops["tags"], serde_json::json!(["ops"]));
}

#[test]
fn list_and_history_report_empty_managed_state_and_apply_history_limits() {
    let env = TestEnv::new();
    let empty_list = json(&env, &["job", "list", "--json"]);
    assert_eq!(empty_list["jobs"], serde_json::json!([]));

    apply(
        &env,
        &[
            "job",
            "create",
            "limited",
            "--schedule",
            "every 1h",
            "--command",
            "true",
        ],
    );
    assert_eq!(
        json(&env, &["job", "history", "limited", "--json"])["runs"],
        serde_json::json!([])
    );

    apply(&env, &["job", "enable", "limited"]);
    for _ in 0..3 {
        apply(&env, &["job", "trigger", "limited"]);
    }
    let history = json(
        &env,
        &["job", "history", "limited", "--limit", "1", "--json"],
    );
    assert_eq!(history["runs"].as_array().unwrap().len(), 1);
}

#[test]
fn history_and_logs_reject_a_non_managed_job_name() {
    let env = TestEnv::new();
    for args in [
        &["job", "history", "missing", "--json"][..],
        &["job", "logs", "missing", "--json"][..],
    ] {
        let output = env.cmd().args(args).output().unwrap();
        assert!(!output.status.success());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["changed"], false);
        assert_eq!(error["error"]["code"], "CW_JOB_NOT_FOUND");
    }
}
