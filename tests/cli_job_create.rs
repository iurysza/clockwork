mod helpers;

use std::fs;

use helpers::TestEnv;
use serde_json::Value;

fn run_json(env: &TestEnv, args: &[&str]) -> (std::process::ExitStatus, Value, String) {
    let output = env.cmd().args(args).output().expect("run clockwork");
    let value = serde_json::from_slice(&output.stdout).expect("one JSON envelope");
    (
        output.status,
        value,
        String::from_utf8(output.stderr).expect("utf8 stderr"),
    )
}

#[test]
fn create_requires_review_then_writes_a_disabled_managed_job() {
    let env = TestEnv::new();
    let args = [
        "job",
        "create",
        "daily-brief",
        "--schedule",
        "every 1h",
        "--command",
        "echo hello",
    ];

    let mut dry_run = args.to_vec();
    dry_run.extend(["--dry-run", "--json"]);
    let (status, preview, _) = run_json(&env, &dry_run);
    assert!(status.success());
    assert_eq!(preview["changed"], false);
    assert_eq!(preview["current_state"], "draft");
    assert_eq!(preview["expected_state"]["type"], "disabled");
    assert_eq!(preview["external_effect"]["type"], "none");
    assert!(
        !env.jobs_dir().exists(),
        "dry run must not create source storage"
    );
    assert!(
        !env.home().join("jobs.json").exists(),
        "dry run must not create runtime state"
    );

    let mut missing_revision = args.to_vec();
    missing_revision.extend(["--yes", "--json"]);
    let (status, error, _) = run_json(&env, &missing_revision);
    assert!(!status.success());
    assert_eq!(error["ok"], false);
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_INVALID_INPUT");

    let revision = preview["revision"].as_str().expect("preview revision");
    let mut apply = args.to_vec();
    apply.extend(["--yes", "--if-revision", revision, "--json"]);
    let (status, result, _) = run_json(&env, &apply);
    assert!(status.success());
    assert_eq!(result["ok"], true);
    assert_eq!(result["changed"], true);
    assert_eq!(result["state"]["type"], "disabled");
    assert_eq!(result["state"]["runtime_generation"], 0);

    let source = fs::read_to_string(env.jobs_dir().join("daily-brief/clockwork.yaml"))
        .expect("managed source exists");
    assert!(source.contains("name: daily-brief"));
    assert!(source.contains("action:\n  command:\n    command: echo hello"));
    assert!(!source.contains("!command"));
    assert!(!source.contains("paused"));
    assert!(!source.contains("enabled"));

    let state: Value = serde_json::from_str(
        &fs::read_to_string(env.home().join("jobs.json")).expect("runtime state exists"),
    )
    .expect("runtime state JSON");
    let job = &state["jobs"]["daily-brief"];
    assert_eq!(job["status"], "paused");
    assert_eq!(job["managed_by"], "managed-job");
    assert!(job["source_revision"].as_str().is_some());
    assert_eq!(job["generation"], 0);
}

#[cfg(unix)]
#[test]
fn create_refuses_a_symlinked_source_directory() {
    use std::os::unix::fs::symlink;

    let env = TestEnv::new();
    let outside = env.home().join("outside");
    fs::create_dir_all(env.jobs_dir()).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, env.jobs_dir().join("linked")).unwrap();

    let (status, error, _) = run_json(
        &env,
        &[
            "job",
            "create",
            "linked",
            "--schedule",
            "every 1h",
            "--command",
            "true",
            "--dry-run",
            "--json",
        ],
    );
    assert!(!status.success());
    assert_eq!(error["error"]["code"], "CW_SOURCE_FAILURE");
    assert!(!outside.join("clockwork.yaml").exists());
}

#[test]
fn create_installs_an_equivalent_hand_written_source_without_rewriting_it() {
    let env = TestEnv::new();
    let source_dir = env.jobs_dir().join("hand-written");
    fs::create_dir_all(&source_dir).unwrap();
    let source = "# keep this comment\nname: hand-written\nschedule: 'every 1h'\naction:\n  command:\n    command: echo hello\n";
    fs::write(source_dir.join("clockwork.yaml"), source).unwrap();

    let args = [
        "job",
        "create",
        "hand-written",
        "--schedule",
        "every 1h",
        "--command",
        "echo hello",
    ];
    let mut preview_args = args.to_vec();
    preview_args.extend(["--dry-run", "--json"]);
    let (status, preview, _) = run_json(&env, &preview_args);
    assert!(status.success());
    let mut apply_args = args.to_vec();
    apply_args.extend([
        "--yes",
        "--if-revision",
        preview["revision"].as_str().unwrap(),
        "--json",
    ]);
    let (status, result, _) = run_json(&env, &apply_args);

    assert!(status.success());
    assert_eq!(result["state"]["type"], "disabled");
    assert_eq!(
        fs::read_to_string(source_dir.join("clockwork.yaml")).unwrap(),
        source
    );
}

#[test]
fn stale_create_revision_does_not_create_storage() {
    let env = TestEnv::new();
    let (status, error, _) = run_json(
        &env,
        &[
            "job",
            "create",
            "stale",
            "--schedule",
            "every 1h",
            "--command",
            "true",
            "--yes",
            "--if-revision",
            "rev_stale",
            "--json",
        ],
    );

    assert!(!status.success());
    assert_eq!(error["changed"], false);
    assert_eq!(error["error"]["code"], "CW_REVISION_CONFLICT");
    assert!(!env.jobs_dir().exists());
    assert!(!env.home().join("jobs.json").exists());
}

#[test]
fn repeated_create_with_the_same_definition_is_a_noop() {
    let env = TestEnv::new();
    let args = [
        "job",
        "create",
        "steady",
        "--schedule",
        "every 1h",
        "--command",
        "true",
    ];

    let (_, preview, _) = run_json(
        &env,
        &[
            "job",
            "create",
            "steady",
            "--schedule",
            "every 1h",
            "--command",
            "true",
            "--dry-run",
            "--json",
        ],
    );
    let revision = preview["revision"].as_str().unwrap();
    let mut create = args.to_vec();
    create.extend(["--yes", "--if-revision", revision, "--json"]);
    assert!(run_json(&env, &create).0.success());

    let (_, retry_preview, _) = run_json(
        &env,
        &[
            "job",
            "create",
            "steady",
            "--schedule",
            "every 1h",
            "--command",
            "true",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(retry_preview["changed"], false);
    let retry_revision = retry_preview["revision"].as_str().unwrap();
    let mut retry = args.to_vec();
    retry.extend(["--yes", "--if-revision", retry_revision, "--json"]);
    let (status, result, _) = run_json(&env, &retry);
    assert!(status.success());
    assert_eq!(result["changed"], false);
}
