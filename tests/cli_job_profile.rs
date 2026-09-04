//! Generic agent-profile coverage: fail-closed resolution, cwd inheritance and
//! override, fixed Pi arguments, optimistic revision pinning, and ownership.
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

fn agents(env: &TestEnv) -> Vec<Value> {
    let output = env
        .cmd()
        .args(["agent", "list", "--json"])
        .output()
        .expect("run clockwork");
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn add_cat_profile(env: &TestEnv, name: &str) {
    env.cmd()
        .args(["agent", "add", name, "--bin", "/bin/cat", "--prompt-stdin"])
        .assert()
        .success();
}

#[test]
fn referenced_profile_must_exist_and_default_profile_resolves() {
    let env = TestEnv::new();

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
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("configured default agent")
    );

    add_cat_profile(&env, "shared");
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
    assert_eq!(
        json(&env, &["job", "status", "default-job", "--json"])["state"]["type"],
        "disabled"
    );
}

#[cfg(unix)]
#[test]
fn generic_pi_profile_forwards_fixed_args_and_applies_cwd_override() {
    use std::os::unix::fs::PermissionsExt;

    let env = TestEnv::new();
    let profile_cwd = env.home().join("profile-project");
    let override_cwd = env.home().join("job-project");
    fs::create_dir_all(&profile_cwd).unwrap();
    fs::create_dir_all(&override_cwd).unwrap();

    let agent = env.home().join("fake-pi.sh");
    fs::write(
        &agent,
        "#!/bin/sh\nprintf 'cwd=%s\\n' \"$PWD\"\nprintf 'args='\nfor arg in \"$@\"; do printf '[%s]' \"$arg\"; done\nprintf '\\nprompt='\ncat\n",
    )
    .unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();

    let session_dir = env.home().join(".local/state/clockwork/pi-sessions/daily");
    env.cmd()
        .args([
            "agent",
            "add",
            "pi-daily",
            "--bin",
            agent.to_str().unwrap(),
            "--cwd",
            profile_cwd.to_str().unwrap(),
            "--prompt-stdin",
            "--arg=--print",
            "--arg=--mode",
            "--arg=json",
            "--arg=--model",
            "--arg=openai-codex/gpt-5.6-sol",
            "--arg=--thinking",
            "--arg=xhigh",
            "--arg=--tools",
            "--arg=read,bash,write",
            "--arg=--approve",
            "--arg=--session-id",
            "--arg=clockwork-daily",
            "--arg=--session-dir",
            &format!("--arg={}", session_dir.display()),
        ])
        .assert()
        .success();

    let profile = agents(&env)
        .into_iter()
        .find(|profile| profile["name"] == "pi-daily")
        .expect("generic Pi profile");
    assert_eq!(profile["cwd"], profile_cwd.to_str().unwrap());

    for (name, cwd) in [
        ("profile-cwd", profile_cwd.as_path()),
        ("override-cwd", override_cwd.as_path()),
    ] {
        let mut create = vec![
            "job",
            "create",
            name,
            "--schedule",
            "every 1h",
            "--prompt",
            "private prompt",
            "--profile",
            "pi-daily",
        ];
        if name == "override-cwd" {
            create.extend(["--cwd", cwd.to_str().unwrap()]);
        }
        apply(&env, &create);
        apply(&env, &["job", "enable", name]);
        apply(&env, &["job", "trigger", name]);

        let log = json(&env, &["job", "logs", name, "--json"])["log"]
            .as_str()
            .unwrap()
            .to_string();
        let resolved_cwd = fs::canonicalize(cwd).unwrap();
        assert!(log.contains(&format!("cwd={}", resolved_cwd.display())));
        assert!(log.contains("[--model][openai-codex/gpt-5.6-sol]"));
        assert!(log.contains("[--tools][read,bash,write]"));
        assert!(log.contains("[--approve]"));
        assert!(log.contains("[--session-id][clockwork-daily]"));
        assert!(log.contains(&format!("[--session-dir][{}]", session_dir.display())));
        assert!(log.contains("prompt=private prompt"));
    }

    let source = fs::read_to_string(env.jobs_dir().join("override-cwd/clockwork.yaml")).unwrap();
    assert!(source.contains(&format!("cwd: {}", override_cwd.display())));
}

#[test]
fn profile_change_after_preview_is_a_revision_conflict() {
    let env = TestEnv::new();
    add_cat_profile(&env, "shared");
    apply(
        &env,
        &[
            "job",
            "create",
            "profiled",
            "--schedule",
            "every 1h",
            "--prompt",
            "hi",
            "--profile",
            "shared",
        ],
    );

    let preview = json(
        &env,
        &[
            "job",
            "update",
            "profiled",
            "--timeout",
            "42",
            "--dry-run",
            "--json",
        ],
    );
    let revision = preview["revision"].as_str().unwrap().to_string();

    env.cmd()
        .args([
            "agent",
            "add",
            "shared",
            "--bin",
            "/bin/cat",
            "--arg=changed",
            "--prompt-stdin",
        ])
        .assert()
        .success();

    let error = apply_expect_error(
        &env,
        &["job", "update", "profiled", "--timeout", "42"],
        &revision,
    );
    assert_eq!(error["error"]["code"], "CW_REVISION_CONFLICT");
    assert_eq!(error["changed"], false);

    let state: Value =
        serde_json::from_str(&fs::read_to_string(env.home().join("jobs.json")).unwrap()).unwrap();
    assert_ne!(state["jobs"]["profiled"]["timeout_seconds"], 42);
}

#[test]
fn create_preview_conflicts_when_target_profile_changes() {
    let env = TestEnv::new();
    add_cat_profile(&env, "target");
    let create = [
        "job",
        "create",
        "profiled",
        "--schedule",
        "every 1h",
        "--prompt",
        "hi",
        "--profile",
        "target",
    ];
    let mut preview_args = create.to_vec();
    preview_args.extend(["--dry-run", "--json"]);
    let preview = json(&env, &preview_args);

    env.cmd()
        .args([
            "agent",
            "add",
            "target",
            "--bin",
            "/bin/cat",
            "--arg=changed",
            "--prompt-stdin",
        ])
        .assert()
        .success();

    let error = apply_expect_error(&env, &create, preview["revision"].as_str().unwrap());
    assert_eq!(error["error"]["code"], "CW_REVISION_CONFLICT");
    assert_eq!(error["changed"], false);
    assert!(!env.jobs_dir().join("profiled/clockwork.yaml").exists());
}

#[test]
fn deleting_a_job_never_deletes_its_generic_profile() {
    let env = TestEnv::new();
    add_cat_profile(&env, "per-job");
    apply(
        &env,
        &[
            "job",
            "create",
            "profiled",
            "--schedule",
            "every 1h",
            "--prompt",
            "hi",
            "--profile",
            "per-job",
        ],
    );
    apply(&env, &["job", "delete", "profiled"]);

    assert!(
        agents(&env)
            .iter()
            .any(|profile| profile["name"] == "per-job")
    );
    assert!(!env.jobs_dir().join("profiled").exists());
}

#[test]
fn job_can_be_deleted_after_its_profile_is_removed() {
    let env = TestEnv::new();
    add_cat_profile(&env, "temporary");
    apply(
        &env,
        &[
            "job",
            "create",
            "profiled",
            "--schedule",
            "every 1h",
            "--prompt",
            "hi",
            "--profile",
            "temporary",
        ],
    );
    env.cmd()
        .args(["agent", "rm", "temporary"])
        .assert()
        .success();

    apply(&env, &["job", "delete", "profiled"]);
    assert!(!env.jobs_dir().join("profiled").exists());
}

#[test]
fn invalid_profile_and_job_cwd_fail_before_mutation() {
    let env = TestEnv::new();
    env.cmd()
        .args([
            "agent",
            "add",
            "bad-cwd",
            "--bin",
            "/bin/cat",
            "--cwd",
            "/definitely/not/a/clockwork-directory",
        ])
        .assert()
        .failure();
    assert!(agents(&env).is_empty());

    add_cat_profile(&env, "valid");
    env.cmd()
        .args([
            "job",
            "create",
            "bad-job-cwd",
            "--schedule",
            "every 1h",
            "--prompt",
            "hi",
            "--profile",
            "valid",
            "--cwd",
            "/definitely/not/a/clockwork-directory",
            "--dry-run",
            "--json",
        ])
        .assert()
        .failure();
    assert!(!env.jobs_dir().join("bad-job-cwd/clockwork.yaml").exists());
}
