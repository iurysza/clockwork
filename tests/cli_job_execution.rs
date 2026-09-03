mod helpers;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

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

fn create_and_enable(env: &TestEnv, name: &str, action: &[&str]) {
    let mut create = vec!["job", "create", name, "--schedule", "every 1h"];
    create.extend_from_slice(action);
    apply(env, &create);
    apply(env, &["job", "enable", name]);
}

fn history(env: &TestEnv, name: &str) -> Value {
    json(env, &["job", "history", name, "--json"])
}

#[test]
fn command_execution_is_repeatable_and_logs_its_output() {
    let env = TestEnv::new();
    create_and_enable(&env, "command", &["--command", "echo command-output"]);

    apply(&env, &["job", "trigger", "command"]);
    apply(&env, &["job", "trigger", "command"]);

    let history = history(&env, "command");
    assert_eq!(history["runs"].as_array().unwrap().len(), 2);
    assert!(
        history["runs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|record| record["status"] == "success")
    );
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(env.home().join("jobs.json")).unwrap())
            .unwrap();
    assert_eq!(state["jobs"]["command"]["run_count"], 2);

    let log = json(&env, &["job", "logs", "command", "--json"]);
    assert!(log["log"].as_str().unwrap().contains("command-output"));
}

#[cfg(unix)]
#[test]
fn command_shell_mode_and_timeout_are_recorded_by_the_normal_executor() {
    let env = TestEnv::new();
    create_and_enable(
        &env,
        "shell",
        &["--command", "echo shell-output | cat", "--shell"],
    );
    apply(&env, &["job", "trigger", "shell"]);
    assert!(
        json(&env, &["job", "logs", "shell", "--json"])["log"]
            .as_str()
            .unwrap()
            .contains("shell-output")
    );

    create_and_enable(&env, "timeout", &["--command", "sleep 2", "--timeout", "1"]);
    apply(&env, &["job", "trigger", "timeout"]);
    assert_eq!(history(&env, "timeout")["runs"][0]["status"], "timeout");
}

#[cfg(unix)]
#[test]
fn prompt_execution_uses_the_registered_profile_and_keeps_text_out_of_previews() {
    use std::os::unix::fs::PermissionsExt;

    let env = TestEnv::new();
    let agent = env.home().join("fake-agent.sh");
    std::fs::write(&agent, "#!/bin/sh\ncat\n").unwrap();
    let mut permissions = std::fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&agent, permissions).unwrap();
    env.cmd()
        .args([
            "agent",
            "add",
            "fake",
            "--bin",
            agent.to_str().unwrap(),
            "--prompt-stdin",
        ])
        .assert()
        .success();

    create_and_enable(
        &env,
        "prompt",
        &["--prompt", "private prompt output", "--profile", "fake"],
    );
    let preview = json(&env, &["job", "trigger", "prompt", "--dry-run", "--json"]);
    assert!(!preview.to_string().contains("private prompt output"));
    apply(&env, &["job", "trigger", "prompt"]);
    assert!(
        json(&env, &["job", "logs", "prompt", "--json"])["log"]
            .as_str()
            .unwrap()
            .contains("private prompt output")
    );
}

#[test]
fn webhook_execution_requires_policy_then_records_the_http_response() {
    let env = TestEnv::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "webhook was not received");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept webhook: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).unwrap();
        let body = "local-webhook-ok";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });

    env.cmd()
        .args(["config", "allow_insecure_http", "true"])
        .assert()
        .success();
    let url = format!("http://{address}/hook?token=secret");
    create_and_enable(&env, "webhook", &["--webhook", &url]);
    let preview = json(&env, &["job", "trigger", "webhook", "--dry-run", "--json"]);
    assert!(!preview.to_string().contains("secret"));
    apply(&env, &["job", "trigger", "webhook"]);
    server.join().unwrap();

    let log = json(&env, &["job", "logs", "webhook", "--json"]);
    assert!(log["log"].as_str().unwrap().contains("HTTP 200"));
    assert!(log["log"].as_str().unwrap().contains("local-webhook-ok"));
}

#[test]
fn trigger_reports_an_internal_executor_failure() {
    let env = TestEnv::new();
    create_and_enable(
        &env,
        "broken-command",
        &["--command", "/definitely/not/a/clockwork-command"],
    );
    let preview = json(
        &env,
        &["job", "trigger", "broken-command", "--dry-run", "--json"],
    );
    let output = env
        .cmd()
        .args([
            "job",
            "trigger",
            "broken-command",
            "--yes",
            "--if-revision",
            preview["revision"].as_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["changed"], true);
    assert_eq!(error["error"]["code"], "CW_MUTATION_FAILED");
    assert_eq!(
        history(&env, "broken-command")["runs"][0]["status"],
        "internal_error"
    );
}

#[test]
fn internal_executor_rejects_fallback_as_a_primary_invocation() {
    let env = TestEnv::new();
    create_and_enable(&env, "boundary", &["--command", "echo should-not-run"]);
    let scheduled_for = Utc::now().to_rfc3339();

    env.cmd()
        .args([
            "_internal",
            "execute",
            "boundary",
            "--scheduled-for",
            &scheduled_for,
            "--trigger",
            "fallback",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "Fallback runs use the dedicated _internal exec-fallback command",
        ));
}
