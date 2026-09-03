# Rust parity baseline

Commit `9f58aa06460066cc0fb18b5f3713188098087105` is the last Clockwork commit that was compared mechanically with the frozen engine source. The exact source-parity statements below apply to that commit.

Later Clockwork commits may change internal architecture. They preserve feature parity through stable external contracts and behavior tests rather than an identical Rust tree.

## Baseline method

1. Export `src/` and `tests/` from the frozen engine commit in `frozen-source.json`.
2. Apply only the upper-case, title-case, and lower-case engine namespace replacements to their matching Clockwork forms.
3. Format both Rust trees with Rust 2024 `rustfmt`.
4. Compare the formatted trees recursively.

The baseline audit found only these differences:

| Path | Difference | Reason |
| --- | --- | --- |
| `src/store/paths.rs` | The default state directory is `~/.local/state/clockwork` instead of the mechanically renamed `~/.clockwork`. | The managed runtime contract requires this path. `CLOCKWORK_HOME` still takes precedence. |
| `src/upgrade/mod.rs` | The release repository is `iurysza/clockwork` instead of the mechanically renamed upstream repository. | Clockwork's future product identity requires this release location. |
| `src/upgrade/binary.rs` | Adds pure tests for the Clockwork archive name and extracting the `clockwork` binary. | The tests prove the renamed binary archive contract without a network call. |
| `src/upgrade/check.rs` | Adds a pure update-hint test. | The test proves Clockwork wording without a network call. |
| `src/upgrade/mod.rs` | Extracts Cargo install-list parsing into a pure helper and adds tests for Clockwork and non-Clockwork entries. | The helper preserves the production branch and makes install-method detection testable without running Cargo. |
| `tests/skill_docs.rs` | Requires at least one valid documented engine command instead of generic `add`, `up`, and `setup` examples. | The merged managed integration skill documents the safe `clockwork-jobs` workflow rather than the engine's generic mutating workflow. |

No other formatted Rust source or Rust test differences existed at `9f58aa0`.

## Run-policy refactor after the baseline

The run-policy refactor intentionally changes the Rust tree:

- `src/model/invocation.rs` gives scheduled and manual runs one internal request type.
- `src/schedule/occurrence.rs` owns due and next-time calculations.
- `src/engine/policy.rs` owns pure dispatch, admission, recovery, and completion decisions.
- `src/engine/dispatcher.rs` and `src/engine/executor.rs` keep filesystem, lock, process, log, and history work.
- `src/engine/action_runner.rs` owns command, prompt, webhook, timeout, and process-control effects.
- The Rust type `InFlightRun` is now `ScheduledClaim`. The saved `in_flight` field and its JSON shape are unchanged.

The refactor preserves the CLI, manifest format, job-state schema, history schema, helper names, managed paths, service label, installer, Rust toolchain, action kinds, and fallback record shape.

Two narrow corrections are intentional:

- Dispatch now saves a consumed scheduled skip even when that tick neither launches a run nor writes an overlap record. The old code changed the in-memory job but did not save it in that case.
- Hidden `_exec --trigger fallback` now fails and points to `_exec-fallback` instead of running the primary action through the wrong path.

## Behavior-parity checks

Use these checks after the baseline:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
npm test
```

On 2026-09-02, the post-refactor suite passes 225 Rust tests and 16 Node tests. The Rust suite covers fixed-time policy transitions and CLI behavior for scheduled runs, manual runs, claims, overlap, skip, recovery, one-time completion, commands, prompts, webhooks, timeouts, history, and fallbacks. The Node suite continues to cover the installer and managed integration contracts.
