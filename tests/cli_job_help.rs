mod helpers;

use helpers::TestEnv;

#[test]
fn root_help_exposes_the_safe_job_interface_not_legacy_runtime_mutations() {
    let env = TestEnv::new();
    let output = env
        .cmd()
        .args(["--help"])
        .output()
        .expect("run clockwork help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("utf8 help");

    assert!(help.contains("job"), "the safe job command must be public");
    for removed in [
        "add",
        "edit",
        "rm",
        "up",
        "down",
        "pause",
        "resume",
        "run",
        "unarchive",
    ] {
        assert!(
            !help
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .any(|command| command == removed),
            "removed public command '{removed}' appeared in root help:\n{help}"
        );
    }
    assert!(
        !help.contains("_internal"),
        "private scheduler and executor routes must not appear in root help:\n{help}"
    );

    let job_help = env
        .cmd()
        .args(["job", "--help"])
        .output()
        .expect("run job help");
    assert!(job_help.status.success());
    let job_help = String::from_utf8(job_help.stdout).expect("utf8 job help");
    for command in [
        "create", "update", "enable", "disable", "delete", "trigger", "validate", "status", "list",
        "history", "logs",
    ] {
        assert!(
            job_help.contains(command),
            "missing job command '{command}'"
        );
    }
}
