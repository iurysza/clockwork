mod helpers;

use chrono::{DateTime, Utc};
use helpers::TestEnv;
use serde_json::Value;

fn json(env: &TestEnv, args: &[&str]) -> Value {
    let output = env.cmd().args(args).output().expect("run clockwork");
    assert!(
        output.status.success(),
        "command failed: {}",
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
        preview["revision"].as_str().expect("revision"),
        "--json",
    ]);
    json(env, &args)
}

fn create(env: &TestEnv) {
    apply(
        env,
        &[
            "job",
            "create",
            "switchable",
            "--schedule",
            "every 1h",
            "--command",
            "true",
        ],
    );
}

#[test]
fn enable_is_the_only_public_activation_path_and_returns_a_future_next_run() {
    let env = TestEnv::new();
    create(&env);

    let result = apply(&env, &["job", "enable", "switchable"]);
    assert_eq!(result["changed"], true);
    assert_eq!(result["state"]["type"], "scheduled");
    let next_run = DateTime::parse_from_rfc3339(
        result["state"]["next_run"]
            .as_str()
            .expect("scheduled next run"),
    )
    .expect("RFC3339 next run")
    .to_utc();
    assert!(
        next_run > Utc::now(),
        "enabled job must have a strict future next run"
    );
    assert_eq!(result["external_effect"]["type"], "future_schedule");
    assert_eq!(result["external_effect"]["action"], "command");

    let status = json(&env, &["job", "status", "switchable", "--json"]);
    assert_eq!(status["activation"], "enabled");
    assert_eq!(status["state"]["type"], "scheduled");
    let verified = DateTime::parse_from_rfc3339(status["state"]["next_run"].as_str().unwrap())
        .unwrap()
        .to_utc();
    assert!(verified > Utc::now());
    assert_eq!(
        verified, next_run,
        "status must retain the activation anchor"
    );

    let no_op = apply(&env, &["job", "enable", "switchable"]);
    assert_eq!(no_op["changed"], false);
    assert_eq!(no_op["state"]["type"], "scheduled");
}

#[test]
fn disable_is_idempotent_and_removes_future_scheduling() {
    let env = TestEnv::new();
    create(&env);
    apply(&env, &["job", "enable", "switchable"]);

    let disabled = apply(&env, &["job", "disable", "switchable"]);
    assert_eq!(disabled["changed"], true);
    assert_eq!(disabled["state"]["type"], "disabled");
    assert_eq!(disabled["external_effect"]["type"], "none");

    let no_op = apply(&env, &["job", "disable", "switchable"]);
    assert_eq!(no_op["changed"], false);
    assert_eq!(no_op["state"]["type"], "disabled");
}
