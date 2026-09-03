# Engine reference

The Rust binary is `clockwork`. It reads `CLOCKWORK_HOME` when set. Otherwise it uses `~/.local/state/clockwork/`.

The standalone integration always sets `CLOCKWORK_HOME` and `CLOCKWORK_BACKEND=none`. Launchd starts one foreground `clockwork daemon` process. It does not use the engine's built-in service backend.

The engine supports imperative jobs, declarative `clockwork.yaml` manifests, command actions, agent prompt actions, HTTPS webhook actions, locks, run history, and explicit pause and resume commands. Managed jobs must use `clockwork-jobs` rather than the engine's direct mutating commands. The managed reconciler supplies the ownership, confirmation, and paused-first rules.

## Run architecture

Clockwork separates run decisions from external effects:

- `src/schedule/occurrence.rs` calculates due and next scheduled times from supplied data.
- `src/engine/policy.rs` decides whether to claim, start, skip, ignore, recover, or complete a run.
- `src/engine/dispatcher.rs` owns dispatch and state locks, state files, history writes, and detached `_exec` launch.
- `src/engine/executor.rs` owns the per-job lock, log creation, completion writes, and fallback launch.
- `src/engine/action_runner.rs` runs commands, prompts, and webhooks.

The policy and occurrence modules do not read the clock, filesystem, environment, or network. The dispatcher and executor supply timestamps, generated IDs, and observed lock state.

Scheduled and manual runs enter the executor through the internal `Invocation` type. This is not a public trigger API. Fallback commands remain follow-up work linked to a failed primary run.

The refactor does not change `jobs.json`, `run-history.jsonl`, CLI commands, action kinds, helper names, managed paths, or service behavior. The saved `in_flight` field still has the same JSON shape. Its Rust type is `ScheduledClaim` because the field records a durable reservation, not proof that execution started.
