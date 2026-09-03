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

fn create(env: &TestEnv, name: &str) {
    let base = [
        "job",
        "create",
        name,
        "--schedule",
        "every 1h",
        "--command",
        "true",
    ];
    let preview = json(
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
    let mut apply = base.to_vec();
    apply.extend([
        "--yes",
        "--if-revision",
        preview["revision"].as_str().unwrap(),
        "--json",
    ]);
    json(env, &apply);
}

#[test]
fn validate_reports_definition_errors_without_losing_their_message() {
    let env = TestEnv::new();
    create(&env, "valid");
    create(&env, "invalid");
    let invalid_source = env.jobs_dir().join("invalid/clockwork.yaml");
    let original = fs::read_to_string(&invalid_source).unwrap();
    fs::write(&invalid_source, original.replace("every 1h", "every 0h")).unwrap();

    let output = env
        .cmd()
        .args(["job", "validate", "--json"])
        .output()
        .expect("run validation");
    assert!(
        !output.status.success(),
        "invalid definitions must fail validation"
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(report["jobs"][0]["job"], "invalid");
    assert_eq!(report["jobs"][0]["valid"], false);
    assert!(
        report["jobs"][0]["errors"][0]
            .as_str()
            .is_some_and(|message| message.contains("Interval must be greater than zero")),
        "validation must retain the original parser error: {report}"
    );
    assert_eq!(report["jobs"][1]["job"], "valid");
    assert_eq!(report["jobs"][1]["valid"], true);
}

#[test]
fn validate_rejects_invalid_job_directory_names() {
    let env = TestEnv::new();
    let invalid_dir = env.jobs_dir().join("-hidden-job");
    fs::create_dir_all(&invalid_dir).unwrap();
    fs::write(
        invalid_dir.join("clockwork.yaml"),
        "name: hidden-job\nschedule: every 1h\naction:\n  command:\n    command: true\n",
    )
    .unwrap();

    let output = env
        .cmd()
        .args(["job", "validate", "--json"])
        .output()
        .expect("run validation");
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_SOURCE_FAILURE");
}

#[test]
fn validate_reports_yaml_parse_errors_for_the_affected_source() {
    let env = TestEnv::new();
    let invalid_dir = env.jobs_dir().join("broken");
    fs::create_dir_all(&invalid_dir).unwrap();
    fs::write(invalid_dir.join("clockwork.yaml"), "name: [unterminated\n").unwrap();

    let output = env
        .cmd()
        .args(["job", "validate", "broken", "--json"])
        .output()
        .expect("run validation");
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(report["jobs"][0]["job"], "broken");
    assert_eq!(report["jobs"][0]["valid"], false);
    assert!(report["jobs"][0]["errors"][0].as_str().is_some());
}
