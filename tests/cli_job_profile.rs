//! Profile ownership coverage: fail-closed prompt profiles, optimistic
//! revision pinning of profile state, coordinator-owned derived profiles,
//! and complete managed-source removal.
mod helpers;

use std::fs;

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

fn apply_expect_error(env: &TestEnv, base: &[&str], revision: &str) -> Value {
    let mut args = base.to_vec();
    args.extend(["--yes", "--if-revision", revision, "--json"]);
    let output = env.cmd().args(&args).output().expect("run clockwork");
    assert!(!output.status.success(), "command unexpectedly succeeded");
    serde_json::from_slice(&output.stdout).expect("one JSON envelope")
}

fn write_pi_source(env: &TestEnv, name: &str, pi_profile: &str) {
    let dir = env.jobs_dir().join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("pi-profile.json"), pi_profile).unwrap();
}

fn valid_pi_profile(env: &TestEnv) -> String {
    format!(
        r#"{{"version":1,"cwd":"{}","model":"anthropic/claude-sonnet-4","thinking":"low","tools":["read"],"approveProjectFiles":false}}"#,
        env.home().display()
    )
}

fn agents(env: &TestEnv) -> Vec<Value> {
    let output = env
        .cmd()
        .args(["agent", "list", "--json"])
        .output()
        .expect("run clockwork");
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn create_pi_job(env: &TestEnv, name: &str, pi_profile: &str) -> Value {
    write_pi_source(env, name, pi_profile);
    apply(
        env,
        &[
            "job",
            "create",
            name,
            "--schedule",
            "every 1h",
            "--prompt",
            "do the thing",
            "--profile",
            &format!("clockwork-pi-{name}"),
        ],
    )
}

#[test]
fn create_owns_the_derived_profile_and_delete_removes_it_with_the_source_directory() {
    let env = TestEnv::new();
    create_pi_job(&env, "pijob", &valid_pi_profile(&env));

    let installed = agents(&env);
    let derived = installed
        .iter()
        .find(|profile| profile["name"] == "clockwork-pi-pijob")
        .expect("coordinator installs the derived profile");
    assert_eq!(derived["args"], serde_json::json!(["--job", "pijob"]));
    assert_eq!(derived["prompt_stdin"], true);
    assert!(derived["bin"].as_str().unwrap().contains("clockwork-pi"));

    apply(&env, &["job", "delete", "pijob"]);

    let remaining = agents(&env);
    assert!(
        remaining
            .iter()
            .all(|profile| profile["name"] != "clockwork-pi-pijob")
    );
    assert!(!env.jobs_dir().join("pijob").exists());
}

#[test]
fn referenced_profile_must_exist_and_delete_keeps_shared_profiles() {
    let env = TestEnv::new();

    // Missing referenced profile fails closed with a recovery hint.
    let output = env
        .cmd()
        .args([
            "job",
            "create",
            "missing",
            "--schedule",
            "every 1h",
            "--prompt",
            "hi",
            "--profile",
            "nope",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "CW_INVALID_INPUT");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("clockwork agent add nope")
    );

    let output = env
        .cmd()
        .args([
            "job",
            "create",
            "no-default",
            "--schedule",
            "every 1h",
            "--prompt",
            "hi",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "CW_INVALID_INPUT");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("configured default agent")
    );

    // A shared registered profile is referenced, never owned.
    env.cmd()
        .args([
            "agent",
            "add",
            "shared",
            "--bin",
            "/bin/cat",
            "--prompt-stdin",
        ])
        .assert()
        .success();
    env.cmd()
        .args(["agent", "default", "shared"])
        .assert()
        .success();
    apply(
        &env,
        &[
            "job",
            "create",
            "default-job",
            "--schedule",
            "every 1h",
            "--prompt",
            "use default",
        ],
    );
    apply(
        &env,
        &[
            "job",
            "create",
            "shared-job",
            "--schedule",
            "every 1h",
            "--prompt",
            "hi",
            "--profile",
            "shared",
        ],
    );
    apply(&env, &["job", "delete", "shared-job"]);
    assert!(
        agents(&env)
            .iter()
            .any(|profile| profile["name"] == "shared")
    );
}

#[test]
fn delete_keeps_a_derived_profile_used_by_another_job() {
    let env = TestEnv::new();
    create_pi_job(&env, "owner", &valid_pi_profile(&env));
    apply(
        &env,
        &[
            "job",
            "create",
            "borrower",
            "--schedule",
            "every 1h",
            "--prompt",
            "hi",
            "--profile",
            "clockwork-pi-owner",
        ],
    );

    apply(&env, &["job", "delete", "owner"]);

    assert!(
        agents(&env)
            .iter()
            .any(|profile| profile["name"] == "clockwork-pi-owner")
    );
    assert!(env.jobs_dir().join("borrower/clockwork.yaml").exists());
}

#[test]
fn malformed_companion_and_non_prompt_companion_fail_closed() {
    let env = TestEnv::new();

    write_pi_source(&env, "badpi", "{not json");
    let output = env
        .cmd()
        .args([
            "job",
            "create",
            "badpi",
            "--schedule",
            "every 1h",
            "--prompt",
            "do the thing",
            "--profile",
            "clockwork-pi-badpi",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "CW_INVALID_INPUT");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pi-profile.json")
    );

    // A hand-written source with a malformed companion validates false.
    let dir = env.jobs_dir().join("badpi");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("clockwork.yaml"),
        "name: badpi\nschedule: every 1h\naction:\n  prompt:\n    profile: clockwork-pi-badpi\n    text: do the thing\n",
    )
    .unwrap();
    fs::write(dir.join("pi-profile.json"), "{not json").unwrap();

    let output = env
        .cmd()
        .args(["job", "validate", "badpi", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(report["jobs"][0]["valid"], false);
    assert!(
        report["jobs"][0]["errors"][0]
            .as_str()
            .unwrap()
            .contains("pi-profile.json")
    );

    // pi-profile.json next to a command action is rejected too.
    write_pi_source(&env, "cmdjob", &valid_pi_profile(&env));
    let output = env
        .cmd()
        .args([
            "job",
            "create",
            "cmdjob",
            "--schedule",
            "every 1h",
            "--command",
            "echo hi",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("only allowed for prompt jobs")
    );
}

#[test]
fn unmanaged_derived_profile_collision_rejects_the_operation() {
    let env = TestEnv::new();
    env.cmd()
        .args([
            "agent",
            "add",
            "clockwork-pi-pijob",
            "--bin",
            "/bin/cat",
            "--prompt-stdin",
        ])
        .assert()
        .success();

    write_pi_source(&env, "pijob", &valid_pi_profile(&env));
    let output = env
        .cmd()
        .args([
            "job",
            "create",
            "pijob",
            "--schedule",
            "every 1h",
            "--prompt",
            "do the thing",
            "--profile",
            "clockwork-pi-pijob",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "CW_INVALID_INPUT");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not owned by job 'pijob'")
    );
}

#[test]
fn profile_change_after_preview_is_a_revision_conflict() {
    let env = TestEnv::new();
    create_pi_job(&env, "pijob", &valid_pi_profile(&env));

    let preview = json(
        &env,
        &[
            "job",
            "update",
            "pijob",
            "--timeout",
            "42",
            "--dry-run",
            "--json",
        ],
    );
    let revision = preview["revision"].as_str().unwrap().to_string();

    // A profile change after preview must move the optimistic revision.
    env.cmd()
        .args(["agent", "rm", "clockwork-pi-pijob"])
        .assert()
        .success();

    let error = apply_expect_error(
        &env,
        &["job", "update", "pijob", "--timeout", "42"],
        &revision,
    );
    assert_eq!(error["error"]["code"], "CW_REVISION_CONFLICT");
    assert_eq!(error["changed"], false);

    // Nothing changed: the runtime still carries the old timeout. Public
    // status also fails closed until the managed profile is repaired.
    let state: Value =
        serde_json::from_str(&fs::read_to_string(env.home().join("jobs.json")).unwrap()).unwrap();
    assert_ne!(state["jobs"]["pijob"]["timeout_seconds"], 42);
    let output = env
        .cmd()
        .args(["job", "status", "pijob", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "CW_INTEGRITY_VIOLATION");

    apply(&env, &["job", "enable", "pijob"]);
    assert!(
        agents(&env)
            .iter()
            .any(|profile| profile["name"] == "clockwork-pi-pijob")
    );
    let status = json(&env, &["job", "status", "pijob", "--json"]);
    assert_eq!(status["state"]["type"], "scheduled");
}

#[test]
fn companion_bytes_change_after_create_preview_is_a_revision_conflict() {
    let env = TestEnv::new();
    write_pi_source(&env, "pijob", &valid_pi_profile(&env));

    let preview = json(
        &env,
        &[
            "job",
            "create",
            "pijob",
            "--schedule",
            "every 1h",
            "--prompt",
            "do the thing",
            "--profile",
            "clockwork-pi-pijob",
            "--dry-run",
            "--json",
        ],
    );
    let revision = preview["revision"].as_str().unwrap().to_string();

    // Editing the companion after preview changes the source revision.
    write_pi_source(&env, "pijob", &valid_pi_profile(&env).replace("low", "off"));

    let error = apply_expect_error(
        &env,
        &[
            "job",
            "create",
            "pijob",
            "--schedule",
            "every 1h",
            "--prompt",
            "do the thing",
            "--profile",
            "clockwork-pi-pijob",
        ],
        &revision,
    );
    assert_eq!(error["error"]["code"], "CW_REVISION_CONFLICT");
    assert_eq!(error["changed"], false);
}
