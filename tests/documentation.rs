//! Exercise public documentation examples without starting agents or services.

mod helpers;

use std::collections::BTreeSet;
use std::fs;

use chrono::{Datelike, TimeZone, Timelike, Utc};
use helpers::TestEnv;
use serde_json::Value;

const README: &str = include_str!("../README.md");
const DOCS: &[&str] = &[
    README,
    include_str!("../docs/install.md"),
    include_str!("../docs/jobs.md"),
    include_str!("../docs/agents.md"),
    include_str!("../docs/scheduling.md"),
    include_str!("../docs/releases.md"),
    include_str!("../services/clockwork/README.md"),
    include_str!("../skills/clockwork/SKILL.md"),
    include_str!("../skills/clockwork/reference.md"),
];

fn fenced_blocks<'a>(markdown: &'a str, language: &str) -> Vec<&'a str> {
    markdown
        .split("```")
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .filter_map(|(_, block)| block.split_once('\n'))
        .filter_map(|(label, content)| (label.trim() == language).then_some(content))
        .collect()
}

fn command_json(env: &TestEnv, args: &[String]) -> Value {
    let output = env.cmd().args(args).output().expect("run clockwork");
    assert!(
        output.status.success(),
        "command failed: {:?}\n{}\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("one JSON document")
}

fn apply(env: &TestEnv, args: &[String]) -> Value {
    let mut preview_args = args.to_vec();
    preview_args.extend(["--dry-run".into(), "--json".into()]);
    let preview = command_json(env, &preview_args);
    let mut apply_args = args.to_vec();
    apply_args.extend([
        "--yes".into(),
        "--if-revision".into(),
        preview["revision"].as_str().unwrap().into(),
        "--json".into(),
    ]);
    command_json(env, &apply_args)
}

#[test]
fn fenced_cli_commands_exist_including_job_and_agent_subcommands() {
    let mut commands = BTreeSet::new();
    for doc in DOCS {
        for language in ["sh", "text"] {
            for block in fenced_blocks(doc, language) {
                for (index, _) in block.match_indices("clockwork ") {
                    let mut words = block[index + "clockwork ".len()..].split_whitespace();
                    let Some(command) = words.next() else {
                        continue;
                    };
                    if !command.chars().all(|c| c.is_ascii_lowercase() || c == '-')
                        || command.starts_with('-')
                    {
                        continue;
                    }
                    let mut args = vec![command.to_string()];
                    if matches!(command, "job" | "agent") {
                        if let Some(subcommand) = words.next() {
                            if subcommand.chars().all(|c| c.is_ascii_lowercase()) {
                                args.push(subcommand.to_string());
                            }
                        }
                    }
                    commands.insert(args);
                }
            }
        }
    }
    assert!(!commands.is_empty());
    let env = TestEnv::new();
    for mut args in commands {
        args.push("--help".into());
        env.cmd().args(&args).assert().success();
    }
}

#[test]
fn readme_prompt_job_creates_disabled_then_enables_on_weekdays() {
    let block = fenced_blocks(README, "sh")
        .into_iter()
        .find(|block| block.starts_with("clockwork job create daily-brief"))
        .expect("README prompt example");
    let mut args = shell_words::split(&block.replace("\\\n", "")).expect("shell arguments");
    assert_eq!(args.remove(0), "clockwork");
    let env = TestEnv::new();
    for arg in &mut args {
        if arg == "$PWD" {
            *arg = env.home().to_string_lossy().into_owned();
        }
    }
    env.cmd()
        .args(["agent", "add", "pi", "--bin", "/usr/bin/true"])
        .assert()
        .success();
    assert_eq!(apply(&env, &args)["state"]["type"], "disabled");
    let enable = ["job".into(), "enable".into(), "daily-brief".into()];
    assert_eq!(apply(&env, &enable)["state"]["type"], "scheduled");

    let schedule = args
        .windows(2)
        .find(|pair| pair[0] == "--schedule")
        .expect("schedule flag")[1]
        .clone();
    let cron: cron::Schedule = format!("0 {schedule} *").parse().unwrap();
    // Two complete weeks catch numeric weekday offsets and weekend runs.
    let monday = Utc.with_ymd_and_hms(2026, 9, 7, 0, 0, 0).unwrap();
    let occurrences: Vec<_> = cron.after(&monday).take(10).collect();
    for (index, occurrence) in occurrences.iter().enumerate() {
        assert_eq!(
            occurrence.weekday().num_days_from_monday(),
            u32::try_from(index % 5).unwrap()
        );
        assert_eq!(occurrence.hour(), 9);
        assert_eq!(occurrence.minute(), 0);
    }
    assert!(!env.home().join("run-history.jsonl").exists());
}

#[test]
fn yaml_examples_pass_the_public_source_validator() {
    let mut examples: Vec<&str> = DOCS
        .iter()
        .flat_map(|doc| fenced_blocks(doc, "yaml"))
        .collect();
    examples.extend([
        include_str!("../services/clockwork/templates/jobs/agent-prompt/clockwork.yaml"),
        include_str!("../services/clockwork/templates/jobs/command/clockwork.yaml"),
        include_str!("../services/clockwork/templates/jobs/https-webhook/clockwork.yaml"),
    ]);
    assert!(!examples.is_empty());
    for example in examples {
        let env = TestEnv::new();
        let mut definition: Value = serde_norway::from_str(example).expect("valid YAML");
        if let Some(prompt) = definition["action"].get_mut("prompt") {
            let profile = prompt["profile"].as_str().expect("example profile");
            env.cmd()
                .args(["agent", "add", profile, "--bin", "/usr/bin/true"])
                .assert()
                .success();
            prompt["cwd"] = Value::String(env.home().to_string_lossy().into_owned());
        }
        let name = definition["name"].as_str().expect("example name");
        let directory = env.jobs_dir().join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("clockwork.yaml"),
            serde_norway::to_string(&definition).unwrap(),
        )
        .unwrap();
        env.cmd()
            .args(["job", "validate", name, "--json"])
            .assert()
            .success();
        assert!(!env.home().join("jobs.json").exists());
    }
}
