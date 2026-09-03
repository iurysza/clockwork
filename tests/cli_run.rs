mod helpers;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use helpers::TestEnv;
use predicates::prelude::*;
use serde_json::Value;

#[cfg(unix)]
#[allow(deprecated)]
fn bin_path() -> std::path::PathBuf {
    assert_cmd::cargo::cargo_bin("clockwork")
}

#[cfg(unix)]
fn wait_for_file(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("file was not created: {}", path.display());
}

#[test]
fn manual_run_captures_output() {
    let env = TestEnv::new();

    env.cmd()
        .args([
            "add",
            "every 1h",
            "--run",
            "echo hello-from-run",
            "--name",
            "run-test",
        ])
        .assert()
        .success();

    env.cmd()
        .args(["run", "run-test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Running job run-test"));

    // Check logs contain the output
    env.cmd()
        .args(["logs", "run-test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello-from-run"));
}

#[test]
fn manual_run_increments_count() {
    let env = TestEnv::new();

    env.cmd()
        .args([
            "add",
            "every 1h",
            "--run",
            "echo count",
            "--name",
            "count-job",
        ])
        .assert()
        .success();

    env.cmd().args(["run", "count-job"]).assert().success();
    env.cmd().args(["run", "count-job"]).assert().success();

    env.cmd()
        .args(["get", "count-job", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"run_count\": 2"));
}

#[test]
fn hidden_exec_rejects_fallback_as_a_primary_invocation() {
    let env = TestEnv::new();
    let output = env
        .cmd()
        .args([
            "add",
            "every 1h",
            "--run",
            "echo should-not-run",
            "--name",
            "fallback-boundary",
            "--json",
        ])
        .output()
        .expect("add job");
    let job: Value = serde_json::from_slice(&output.stdout).expect("valid job JSON");
    let job_id = job["id"].as_str().expect("job id");
    let scheduled_for = chrono::Utc::now().to_rfc3339();

    env.cmd()
        .args([
            "_exec",
            job_id,
            "--scheduled-for",
            &scheduled_for,
            "--trigger",
            "fallback",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Fallback runs use the dedicated _exec-fallback command",
        ));
}

#[test]
fn run_nonexistent_job() {
    let env = TestEnv::new();

    env.cmd()
        .args(["run", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn manual_run_of_paused_job_remains_a_noop() {
    let env = TestEnv::new();
    env.cmd()
        .args([
            "add",
            "every 1h",
            "--run",
            "echo should-not-run",
            "--name",
            "paused-run",
        ])
        .assert()
        .success();
    env.cmd().args(["pause", "paused-run"]).assert().success();

    env.cmd().args(["run", "paused-run"]).assert().success();
    env.cmd()
        .args(["get", "paused-run", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"run_count\": 0"));
    env.cmd()
        .args(["history", "paused-run", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));
}

#[test]
fn run_with_shell_mode() {
    let env = TestEnv::new();

    env.cmd()
        .args([
            "add",
            "every 1h",
            "--run",
            "echo shell | cat",
            "--shell",
            "--name",
            "shell-job",
        ])
        .assert()
        .success();

    env.cmd().args(["run", "shell-job"]).assert().success();

    env.cmd()
        .args(["logs", "shell-job"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shell"));
}

#[test]
fn logs_with_specific_run_shows_header() {
    let env = TestEnv::new();

    env.cmd()
        .args([
            "add",
            "every 1h",
            "--run",
            "echo header-test",
            "--name",
            "header-job",
        ])
        .assert()
        .success();

    env.cmd().args(["run", "header-job"]).assert().success();

    let history_output = env
        .cmd()
        .args(["history", "header-job", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let history: Value = serde_json::from_slice(&history_output).expect("valid history JSON");
    let run_id = history
        .as_array()
        .and_then(|records| records.first())
        .and_then(|record| record.get("run_id"))
        .and_then(Value::as_str)
        .expect("history contains a run_id")
        .to_string();

    env.cmd()
        .args(["logs", "header-job", "--run", &run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("Run: {run_id} at ")))
        .stdout(predicate::str::contains("Logs:"))
        .stdout(predicate::str::contains("header-test"));
}

#[cfg(unix)]
#[test]
fn manual_run_records_timeout() {
    let env = TestEnv::new();
    env.cmd()
        .args([
            "add",
            "every 1h",
            "--run",
            "sleep 5",
            "--timeout",
            "1",
            "--name",
            "timeout-run",
        ])
        .assert()
        .success();

    env.cmd().args(["run", "timeout-run"]).assert().success();
    env.cmd()
        .args(["history", "timeout-run", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\": \"timeout\""));
}

#[cfg(unix)]
#[test]
fn prompt_action_uses_registered_agent_and_captures_output() {
    use std::os::unix::fs::PermissionsExt;

    let env = TestEnv::new();
    let agent = env.home().join("fake-agent.sh");
    std::fs::write(&agent, "#!/bin/sh\ncat\n").expect("write fake agent");
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
    env.cmd()
        .args([
            "add",
            "every 1h",
            "--prompt",
            "prompt-adapter-output",
            "--agent",
            "fake",
            "--name",
            "prompt-run",
        ])
        .assert()
        .success();

    env.cmd().args(["run", "prompt-run"]).assert().success();
    env.cmd()
        .args(["logs", "prompt-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("prompt-adapter-output"));
}

#[test]
fn webhook_action_uses_local_http_server_and_captures_response() {
    let env = TestEnv::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local server");
    listener
        .set_nonblocking(true)
        .expect("set local server nonblocking");
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
        stream
            .set_nonblocking(false)
            .expect("restore blocking request reads");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("set request timeout");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).expect("read request");
        let body = "local-webhook-ok";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("write response");
    });

    env.cmd()
        .args(["config", "allow_insecure_http", "true"])
        .assert()
        .success();
    env.cmd()
        .args([
            "add",
            "every 1h",
            "--webhook",
            &format!("http://{address}/hook"),
            "--name",
            "webhook-run",
        ])
        .assert()
        .success();
    env.cmd().args(["run", "webhook-run"]).assert().success();
    server.join().expect("local server should finish");

    env.cmd()
        .args(["logs", "webhook-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("HTTP 200"))
        .stdout(predicate::str::contains("local-webhook-ok"));
}

#[cfg(unix)]
#[test]
fn concurrent_manual_run_records_overlap() {
    let env = TestEnv::new();
    let started = env.home().join("started");
    let command = format!("touch {}; sleep 1", started.display());
    env.cmd()
        .args([
            "add",
            "every 1h",
            "--run",
            &command,
            "--shell",
            "--name",
            "manual-overlap",
        ])
        .assert()
        .success();

    let mut first = std::process::Command::new(bin_path())
        .env("CLOCKWORK_HOME", env.home())
        .env("CLOCKWORK_BACKEND", "none")
        .args(["run", "manual-overlap"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start first manual run");
    wait_for_file(&started);

    env.cmd().args(["run", "manual-overlap"]).assert().success();
    assert!(first.wait().expect("wait for first run").success());

    let history_output = env
        .cmd()
        .args(["history", "manual-overlap", "--json"])
        .output()
        .expect("read history");
    let history: Vec<Value> = serde_json::from_slice(&history_output.stdout).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        history
            .iter()
            .filter(|record| record["status"] == "skipped_overlap")
            .count(),
        1
    );
}
