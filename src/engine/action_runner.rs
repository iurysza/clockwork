use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::model::action::{Action, HttpMethod};
use crate::model::job::Job;
use crate::store::config::load_config;

use super::policy::ActionExit;

const MAX_WEBHOOK_RESPONSE_LOG_BYTES: u64 = 256 * 1024;

pub fn execute(job: &Job, log_file: std::fs::File) -> Result<ActionExit> {
    match &job.action {
        Action::Run {
            command,
            shell,
            workdir,
        } => execute_run_action(
            command,
            *shell,
            workdir.as_deref(),
            job.timeout_seconds,
            log_file,
        ),
        Action::Prompt { text, agent, cwd } => execute_prompt_action(
            text,
            agent.as_deref(),
            cwd.as_deref(),
            job.timeout_seconds,
            log_file,
        ),
        Action::Webhook {
            url,
            method,
            headers,
            body,
        } => execute_webhook_action(
            url,
            *method,
            headers,
            body.as_deref(),
            job.timeout_seconds,
            log_file,
        ),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn execute_run_action(
    command: &str,
    shell: bool,
    workdir: Option<&str>,
    timeout_seconds: u64,
    log_file: std::fs::File,
) -> Result<ActionExit> {
    let mut command = if shell {
        let mut shell_command = Command::new("/bin/sh");
        shell_command.args(["-lc", command]);
        shell_command
    } else {
        let arguments = shell_words::split(command).context("failed to parse command")?;
        if arguments.is_empty() {
            anyhow::bail!("empty command");
        }
        let mut direct_command = Command::new(&arguments[0]);
        if arguments.len() > 1 {
            direct_command.args(&arguments[1..]);
        }
        direct_command
    };

    if let Some(directory) = workdir {
        command.current_dir(directory);
    }

    let stdout_file = log_file.try_clone()?;
    let stderr_file = log_file.try_clone()?;
    command.stdout(Stdio::from(stdout_file));
    command.stderr(Stdio::from(stderr_file));
    isolate_child_process(&mut command);

    let mut child = command.spawn().context("failed to spawn command")?;
    wait_for_child(&mut child, Duration::from_secs(timeout_seconds))
}

#[allow(clippy::needless_pass_by_value)]
fn execute_prompt_action(
    prompt_text: &str,
    agent_name: Option<&str>,
    cwd_override: Option<&str>,
    timeout_seconds: u64,
    log_file: std::fs::File,
) -> Result<ActionExit> {
    let config = load_config()?;
    let agent_key = agent_name
        .map(std::string::ToString::to_string)
        .or(config.default_agent.clone())
        .context(
            "No agent specified and no default agent configured.\n\
             Run: clockwork agent add <name> --bin <path>",
        )?;

    let profile = config.agents.get(&agent_key).with_context(|| {
        format!(
            "Agent '{agent_key}' not found in config.\n\
             Run: clockwork agent list"
        )
    })?;

    let mut command = Command::new(&profile.bin);
    command.args(&profile.args);
    if let Some(directory) = cwd_override.or(profile.cwd.as_deref()) {
        command.current_dir(crate::util::path::resolve_directory(directory)?);
    }

    let stdout_file = log_file.try_clone()?;
    let stderr_file = log_file.try_clone()?;
    command.stdout(Stdio::from(stdout_file));
    command.stderr(Stdio::from(stderr_file));
    isolate_child_process(&mut command);

    if profile.prompt_stdin {
        command.stdin(Stdio::piped());
    } else {
        command.arg(prompt_text);
        command.stdin(Stdio::null());
    }

    let mut child = command.spawn().context("failed to spawn agent")?;

    if profile.prompt_stdin {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt_text.as_bytes()).ok();
            drop(stdin);
        }
    }

    wait_for_child(&mut child, Duration::from_secs(timeout_seconds))
}

#[allow(clippy::too_many_lines)]
fn execute_webhook_action(
    url: &str,
    method: HttpMethod,
    headers: &[(String, String)],
    body: Option<&str>,
    timeout_seconds: u64,
    mut log_file: std::fs::File,
) -> Result<ActionExit> {
    let config = load_config()?;

    let parsed = url::Url::parse(url).context("invalid webhook URL")?;
    if parsed.scheme() == "http" && !config.allow_insecure_http {
        anyhow::bail!(
            "HTTP webhooks are blocked by default.\n\
             To allow: clockwork config allow_insecure_http true"
        );
    }
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        anyhow::bail!("Only http:// and https:// webhook URLs are supported.");
    }

    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_seconds)))
        .build();
    let agent: ureq::Agent = agent_config.into();

    if let Some(body_value) = body {
        if body_value.len() > 1_048_576 {
            anyhow::bail!("Webhook body exceeds maximum size of 1 MiB.");
        }
    }

    let response = match (method, body) {
        (HttpMethod::Get, _) => {
            let mut request = agent.get(url);
            for (key, value) in headers {
                request = request.header(key, value);
            }
            request.call()
        }
        (HttpMethod::Delete, None | Some(_)) => {
            let mut request = agent.delete(url);
            for (key, value) in headers {
                request = request.header(key, value);
            }
            request.call()
        }
        (HttpMethod::Post, body) => {
            let mut request = agent.post(url);
            for (key, value) in headers {
                request = request.header(key, value);
            }
            if let Some(body_value) = body {
                request
                    .header("Content-Type", "application/json")
                    .send(body_value)
            } else {
                request.send("")
            }
        }
        (HttpMethod::Put, body) => {
            let mut request = agent.put(url);
            for (key, value) in headers {
                request = request.header(key, value);
            }
            if let Some(body_value) = body {
                request
                    .header("Content-Type", "application/json")
                    .send(body_value)
            } else {
                request.send("")
            }
        }
        (HttpMethod::Patch, body) => {
            let mut request = agent.patch(url);
            for (key, value) in headers {
                request = request.header(key, value);
            }
            if let Some(body_value) = body {
                request
                    .header("Content-Type", "application/json")
                    .send(body_value)
            } else {
                request.send("")
            }
        }
    };

    match response {
        Ok(mut response) => {
            let status = response.status().as_u16();
            let mut body_reader = response
                .body_mut()
                .with_config()
                .limit(MAX_WEBHOOK_RESPONSE_LOG_BYTES)
                .reader();
            let mut body_bytes = Vec::new();
            let body_read_error = body_reader.read_to_end(&mut body_bytes).err();
            let body_text = String::from_utf8_lossy(&body_bytes);

            writeln!(log_file, "HTTP {status}").ok();
            writeln!(log_file, "{body_text}").ok();
            if let Some(error) = body_read_error {
                writeln!(log_file, "[response body truncated or unreadable: {error}]").ok();
            }

            if (200..300).contains(&status) {
                Ok(ActionExit::Exited { code: Some(0) })
            } else {
                Ok(ActionExit::Exited {
                    code: Some(i32::from(status)),
                })
            }
        }
        Err(error) => {
            writeln!(log_file, "Webhook error: {error}").ok();
            let message = error.to_string();
            if message.contains("timed out") || message.contains("timeout") {
                Ok(ActionExit::TimedOut)
            } else {
                Ok(ActionExit::Exited { code: Some(1) })
            }
        }
    }
}

pub(crate) fn wait_for_child(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<ActionExit> {
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(ActionExit::Exited {
                code: status.code(),
            });
        }
        if start.elapsed() >= timeout {
            kill_process(child);
            return Ok(ActionExit::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn kill_process(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(child.id()) {
            if pid > 0 {
                let _ = unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
pub(crate) fn isolate_child_process(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub(crate) fn isolate_child_process(_command: &mut Command) {}
