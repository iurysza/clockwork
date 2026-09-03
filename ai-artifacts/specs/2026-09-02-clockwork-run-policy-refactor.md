# Refactor Clockwork run policy into a functional core

## Summary

Refactor the scheduled and manual primary-run path around pure decisions and a small imperative shell. Keep the CLI, persisted JSON, managed integration, action kinds, and runtime topology unchanged.

The recommendation adds four internal modules:

- `src/model/invocation.rs` for typed primary invocations and attempts.
- `src/schedule/occurrence.rs` for shared occurrence calculations.
- `src/engine/policy.rs` for dispatch, admission, recovery, and completion decisions.
- `src/engine/action_runner.rs` for command, prompt, and webhook effects.

`dispatcher.rs` and `executor.rs` remain the shells. They own clocks, IDs, locks, files, process launch, HTTP, logs, and history. The new core receives those values as data and returns a plan.

Do not add a trigger command, plugin API, agent SDK, session wake, Pi steering, or password capability in this refactor. A later ingress can construct the same internal `Invocation` after a real second caller exists.

Current validation is green: `cargo test --all-targets --all-features` passes 203 Rust tests, and `npm test` passes 16 Node tests on 2026-09-02.

The refactor ends exact source-tree parity with the frozen engine. Before implementation, treat commit `9f58aa0` as the audited parity baseline. Update `docs/provenance/rust-parity.md` to distinguish that historical source parity from post-refactor behavior parity. Do not leave its current "No other formatted Rust source or Rust test differences remain" claim in the present tense after the tree changes.

## Current state

`src/engine/dispatcher.rs` is 458 lines. It combines due-time policy, claim recovery, overlap policy, state locks, state writes, history writes, detached process launch, archival, and log cleanup.

`src/engine/executor.rs` is 794 lines. It combines invocation admission, overlap handling, action dispatch, command and agent process control, HTTP, timeout handling, state completion, history, failure logs, and fallback launch.

The current scheduled flow is:

```text
clockwork daemon or clockwork _dispatch
  -> dispatcher::dispatch(Utc::now())
  -> acquire dispatch lock
  -> recover stale claims
  -> load jobs
  -> process_job for each job
  -> compute due occurrence
  -> save Job.in_flight claim under state lock
  -> spawn detached clockwork _exec
  -> executor::exec_job_with_run_id
  -> validate active state and claim identity
  -> acquire per-job lock
  -> create log
  -> execute command, prompt, or webhook
  -> classify result
  -> save job completion under state lock
  -> append RunRecord
  -> optionally spawn clockwork _exec-fallback
```

The current manual flow is:

```text
clockwork run <job>
  -> commands::run::execute
  -> executor::exec_job
  -> active-state and overlap checks
  -> same action and completion code as a scheduled run
```

`docs/provenance/rust-parity.md` proves that commit `9f58aa0` differs from the frozen Rust source only where the managed Clockwork integration requires it. The roadmap requires feature parity before new abstractions, and that condition is met at the baseline commit. After this refactor, the full test suite and stable external contracts become the parity evidence.

The current design already has useful seams, but they are implicit:

- `Job.in_flight` is a scheduled claim persisted before process launch.
- `Trigger` stores scheduled, manual, or fallback provenance in history.
- The per-job file lock prevents concurrent action execution.
- `Action` selects a command, prompt, or webhook effect.
- `services/clockwork/pi-launcher.mjs` is an external action adapter.

The main concerns are:

1. Policy calls `Utc::now()`, `new_run_id()`, file stores, locks, process APIs, and HTTP directly. Fixed-input tests cannot cover the full transition.
2. Dispatch and output calculate occurrences through separate code paths. The paths can drift.
3. `Trigger::Fallback` looks like an invocation source, but a fallback is a follow-up to a failed primary run.
4. The state file is saved before history is appended. A failure between those writes can leave a completed state without a history line. The refactor must expose this boundary but must not pretend that the two files form one transaction.
5. Manual invocation policy is underspecified. Today a paused manual run returns success without executing, and a manual run advances `last_scheduled_at`. Changing those rules is a product decision, not a refactor detail.
6. Fallback concurrency counts lock-file names rather than proven held locks. Old `fallback-*.lock` files can make the count stale. This is a separate correctness defect and does not belong in the primary-run refactor.

## Goals

- Move the rules for selecting due jobs, deciding whether a run may start, recovering abandoned claims, and applying run results into pure functions. Keep file, lock, process, and network work outside those functions.
- Send scheduled and manual runs through one internal `Invocation` type.
- Keep fallback commands separate from runs of the primary action.
- Use the same schedule calculations when dispatching jobs and showing their next run.
- Keep `dispatcher.rs` focused on locks, files, and child processes.
- Keep `executor.rs` focused on running an accepted job and saving its result.
- Preserve every current feature and managed safety rule.
- Make it easy to add one more local caller later without adding a plugin system.
- Test decisions with fixed times and ordinary data. Keep CLI tests for real files, locks, and processes.
- Keep `9f58aa0` as the last commit that exactly matches the frozen source after the approved Clockwork changes.

## Non-goals

This refactor does not add new product behavior:

- Do not add a public `clockwork trigger` command or save an external source label.
- Do not add plugins, callbacks, sockets, or an agent SDK.
- Do not add another `Action` kind.
- Do not add session wake, live Pi steering, or password injection.
- Do not change the agents repository or the live scheduler.
- Do not change the JSON formats for job state or run history.
- Do not try to combine `jobs.json` and `run-history.jsonl` into one transaction.
- Do not redesign fallback limits, service management, reconciliation, upgrades, or installation.
- Do not add generic traits for storage, clocks, ID generation, or action execution.

## Invariants and constraints

- Each job keeps one schedule and one primary action. Once Clockwork accepts a run, it uses the job definition that it loaded for that run.
- A schedule creates scheduled times. A manual run is not a scheduled time.
- Keep the saved `in_flight` field unchanged. Rename its Rust type to `ScheduledClaim` because it records a reservation, not a started run.
- Save a scheduled claim before starting `_exec`.
- Start `_exec` only when its run ID and scheduled time match the saved claim.
- Hold the per-job lock until the external action finishes.
- Stopping the daemon must not stop a detached run that has already started.
- Preserve current overlap, skip, stale-claim, one-time completion, failure-count, and fallback behavior. Change those rules only after a separate product decision.
- Keep the saved `RunRecord.trigger` values `scheduled`, `manual`, and `fallback`.
- Keep fallback as a follow-up to a failed run. Link it with `failed_run_id`; do not treat it as another request to run the primary action.
- Save the completed job state before appending run history, as Clockwork does today. Report either write failure clearly.
- A run record proves what Clockwork did. It does not prove that an email, webhook, or other external delivery succeeded.
- Do not put prompts, webhook bodies, credentials, or provider payloads into new logs or errors.
- Add no dependency and no long-running process.
- Update `docs/provenance/rust-parity.md` to say that exact source matching ends at `9f58aa0`. After the refactor, passing behavior tests and unchanged external contracts prove parity.

## Alternatives

### Option 1: leave the code as it is

Keep `Job`, `InFlightRun`, `Trigger`, and `RunRecord` as the only run-lifecycle types. Keep all decisions and effects inside `dispatcher.rs` and `executor.rs`.

`dispatcher::dispatch` and `executor::exec_job_with_run_id` remain the main entry points. Timeouts still stop the action process, stopping the daemon still leaves detached runs alive, and state and history remain separate writes. CLI tests remain the main test boundary.

This has the lowest immediate risk because no code moves. It also leaves schedule calculations duplicated and keeps behavior rules mixed with files, locks, processes, and HTTP. A future caller would need to call `exec_job` directly or repeat its checks.

### Option 2: pure decision functions with small file and process wrappers

Add `Invocation`, `RunDecision`, `DispatchPlan`, `RunOutcome`, and `CompletionPlan`. Pure functions receive a job, fixed times, generated IDs, and whether a lock is busy. They return a decision and the data that must be saved or acted on.

`dispatcher.rs` keeps the dispatch and state locks, file access, ID generation, history writes, and `_exec` process launch. `executor.rs` keeps the per-job lock, logs, state and history writes, and fallback launch. `action_runner.rs` runs commands, prompts, and webhooks.

The main path becomes:

```text
CLI or dispatcher
  -> Invocation
  -> decide whether to start, skip, or ignore
  -> run the action when accepted
  -> classify the result
  -> calculate the new job state and run record
  -> save state and history
  -> start fallback when needed
```

The decision functions return clear results such as `Start`, `Skip`, or `Ignore`. File, process, and network failures remain normal errors with their original cause. State and history remain separate writes in their current order. Timeout handling and daemon shutdown behavior do not change.

Pure tests cover the rules without mocks. CLI tests continue to cover real files, locks, child processes, and output.

This option adds four focused modules and a small amount of mapping code. The main risk is changing behavior while moving it. Add tests before moving each rule.

### Option 3: add interfaces for every dependency

Add a `RunService` that uses `JobRepository`, `HistoryRepository`, `Clock`, `IdGenerator`, `ActionExecutor`, and `RunLauncher` traits. Provide separate implementations for the filesystem, processes, HTTP, and tests.

Commands and the daemon would call `RunService`, which would call those interfaces, which would then reach the filesystem, processes, or HTTP. State and history would still be separate writes. Cancellation would also need to pass through the new interfaces.

This makes each dependency replaceable. It also adds many interfaces and wiring types to a small command-line tool. Tests would rely more on mocks and call expectations. Clockwork does not need alternate stores or runtimes today, so this option solves problems we do not have.

Do not build a dynamic plugin host. It would need discovery, versioning, process lifecycle, and permission rules before Clockwork has a second caller or adapter.

## Recommendation

Choose Option 2.

Earlier research said to wait for a second caller before adding another way to enter Clockwork. This refactor does not add one. It gives the existing scheduled and manual paths one internal handoff and makes their rules testable.

Keep executable actions as the integration boundary. This preserves the Unix model: Clockwork decides whether a job may run, then a command performs the work.

Preserve current behavior during the refactor. In particular, do not decide how paused or one-time jobs should behave when run manually. Keep today's behavior until that product decision is made separately.

Do not create an ADR for this internal change. Create one later if Clockwork adds a public invocation command, saves external source labels, changes its state model, or controls live agent sessions.

## Domain model and types

### Scheduled claim

Rename only the Rust type. Keep the serialized field and object shape.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledClaim {
    pub run_id: String,
    pub scheduled_for: DateTime<Utc>,
    pub claimed_at: DateTime<Utc>,
}

pub struct Job {
    // Existing fields stay unchanged.
    pub in_flight: Option<ScheduledClaim>,
}
```

`ScheduledClaim` may cross the state-store boundary. It must not be treated as proof that `_exec` acquired the job lock or started the action.

### Primary invocation

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationSource {
    Scheduled {
        occurrence_at: DateTime<Utc>,
    },
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub job_id: String,
    pub run_id: String,
    pub requested_at: DateTime<Utc>,
    pub source: InvocationSource,
}

#[derive(Debug, thiserror::Error)]
pub enum InvocationInputError {
    #[error("missing run ID for scheduled invocation")]
    MissingScheduledRunId,
    #[error("fallback uses the dedicated fallback command")]
    FallbackIsNotPrimary,
}
```

`Invocation` is internal application data. For a scheduled invocation, `run_id` is the scheduled claim identity. Raw CLI strings, arbitrary source labels, action bodies, and adapter-specific session handles must not cross this boundary.

A future external caller may add a bounded variant after its contract exists. Do not add an unused `External` variant now.

### Admission

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionAvailability {
    Available,
    Busy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IgnoreReason {
    InactiveJob,
    StaleOrDuplicateClaim,
    ScheduledLockBusy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Overlap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAttempt {
    pub job_id: String,
    pub run_id: String,
    pub requested_at: DateTime<Utc>,
    pub source: InvocationSource,
}

impl RunAttempt {
    pub fn recorded_for(&self) -> DateTime<Utc> {
        match self.source {
            InvocationSource::Scheduled { occurrence_at } => occurrence_at,
            InvocationSource::Manual => self.requested_at,
        }
    }
}

#[derive(Debug, Clone)]
pub enum RunDecision {
    Start(RunAttempt),
    Skip { reason: SkipReason, record: RunRecord },
    Ignore(IgnoreReason),
}

pub fn decide_run(
    job: &Job,
    invocation: &Invocation,
    availability: ExecutionAvailability,
    observed_at: DateTime<Utc>,
) -> RunDecision;
```

The shell must hold the successful per-job lock guard while it handles `Start`. A busy manual invocation returns a skipped-overlap record. A busy scheduled invocation keeps the current claim for later recovery, matching current behavior.

### Dispatch planning

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimedExecution {
    NotRunning,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchEffect {
    Launch(Invocation),
    RecordSkippedOverlap { scheduled_for: DateTime<Utc> },
}

#[derive(Debug, Clone)]
pub struct DispatchPlan {
    pub job: Job,
    pub changed: bool,
    pub effects: Vec<DispatchEffect>,
}

pub fn plan_dispatch(
    job: &Job,
    now: DateTime<Utc>,
    claimed_execution: ClaimedExecution,
    proposed_run_id: String,
) -> Result<DispatchPlan, OccurrenceError>;
```

The shell generates `proposed_run_id`. The core may ignore it when no claim is needed. The shell generates run IDs for skipped-overlap records when it projects `DispatchEffect` into `RunRecord`.

### Claim recovery

```rust
#[derive(Debug, Clone)]
pub enum ClaimRecovery {
    Keep,
    Recover { job: Job, record: RunRecord },
}

pub fn recover_claim(
    job: &Job,
    now: DateTime<Utc>,
    claimed_execution: ClaimedExecution,
    grace: chrono::Duration,
) -> ClaimRecovery;
```

The shell passes the existing 10-second compatibility grace from `dispatcher.rs`. Recovery clears the claim and writes an internal-error record without incrementing `run_count`, matching current behavior.

### Outcome and completion

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionExit {
    Exited { code: Option<i32> },
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Success { exit_code: i32 },
    Failed { exit_code: Option<i32> },
    Timeout,
    InternalError { safe_message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTimes {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct FailureRequest {
    pub job_id: String,
    pub failed_run_id: String,
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub log_path: String,
    pub recorded_for: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CompletionPlan {
    pub job: Option<Job>,
    pub record: RunRecord,
    pub failure: Option<FailureRequest>,
}

pub fn classify_outcome(
    result: Result<ActionExit, ActionRunError>,
) -> RunOutcome;

pub fn complete_run(
    current_job: Option<&Job>,
    attempt: &RunAttempt,
    outcome: &RunOutcome,
    times: RunTimes,
    log_path: String,
) -> CompletionPlan;
```

`current_job` is loaded again at completion because the job may be edited or removed during execution. The action still uses the admitted snapshot, matching current behavior.

`safe_message` may contain a failure category and safe path or program name. It must not contain prompt text, webhook bodies, authorization headers, credentials, or raw provider responses.

### Occurrence calculations

```rust
#[derive(Debug, thiserror::Error)]
pub enum OccurrenceError {
    #[error("invalid stored cron expression")]
    InvalidCron(#[source] cron::error::Error),
}

pub fn latest_due(
    schedule: &JobSchedule,
    after: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, OccurrenceError>;

pub fn due_after(
    schedule: &JobSchedule,
    after: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Vec<DateTime<Utc>>, OccurrenceError>;

pub fn next_after(
    schedule: &JobSchedule,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, OccurrenceError>;
```

`dispatcher.rs` and `output/format.rs` use these functions. Callers supply `now`; occurrence code must not read the clock.

## Interfaces and APIs

### Public CLI

No public command or flag changes.

- `clockwork run <job>` remains the manual entrypoint.
- `clockwork daemon` and hidden `_dispatch` remain scheduled entrypoints.
- Hidden `_exec` keeps `job_id`, `--scheduled-for`, `--trigger`, and `--run-id` for compatibility.
- Hidden `_exec-fallback` remains separate.
- Human output, JSON output, and exit-code classification remain stable.

`commands::exec` parses raw strings once and constructs `Invocation`. `commands::run` resolves the job ID, supplies one timestamp and run ID, then constructs a manual `Invocation`.

### Persisted APIs

No serialized shape changes.

- `jobs.json` keeps schema version 2 and field name `in_flight`.
- `run-history.jsonl` keeps `trigger` and all current values.
- Log paths and file layouts remain unchanged.
- Managed ownership, Pi session, and delivery-receipt files remain untouched.

### Internal orchestration

```rust
pub fn execute_invocation(
    invocation: Invocation,
) -> anyhow::Result<ExecutionDisposition>;

pub enum ExecutionDisposition {
    Completed(RunOutcome),
    Skipped(RunRecord),
    Ignored(IgnoreReason),
}
```

The command layer maps `ExecutionDisposition` to existing output and exit behavior. Do not collapse it back to the current `bool`, which hides whether an action ran.

## Boundaries and adapters

### Functional core

`engine/policy.rs` and `schedule/occurrence.rs` may depend on model types, `chrono` values passed by callers, and pure cron calculations. They must not access:

- `Utc::now()`;
- `new_run_id()`;
- environment variables;
- file stores or locks;
- logs;
- processes;
- HTTP;
- Pi or another agent runtime.

### Dispatcher shell

`dispatcher.rs` owns:

- the dispatch lock and state lock;
- per-job lock probes for existing claims;
- state loads and atomic state saves;
- run-ID generation;
- history append for overlap and recovery records;
- detached `_exec` launch;
- archive timing and log cleanup.

The shell executes effects only after it persists the `DispatchPlan.job` when `changed` is true.

### Executor shell

`executor.rs` owns:

- loading the admitted job snapshot;
- acquiring and retaining the per-job lock;
- log-file creation;
- calling `action_runner`;
- loading current state again after the effect;
- applying `CompletionPlan.job` under the state lock;
- appending `CompletionPlan.record`;
- appending the existing failure log and spawning `_exec-fallback`.

Do not hold the state lock across an action.

### Action runner

```rust
#[derive(Debug, thiserror::Error)]
pub enum ActionRunError {
    #[error("invalid command input")]
    InvalidCommand,
    #[error("action process could not start")]
    Spawn(#[source] std::io::Error),
    #[error("agent configuration is invalid")]
    AgentConfig(#[source] anyhow::Error),
    #[error("webhook request failed")]
    Webhook(#[source] anyhow::Error),
}

pub fn execute(
    job: &Job,
    log_file: std::fs::File,
) -> Result<ActionExit, ActionRunError>;
```

`action_runner.rs` owns the existing command splitting, shell invocation, agent profile lookup, prompt delivery, HTTP request, timeout wait, process-group kill, and response logging. It is an imperative adapter, not a trait or plugin registry.

### External action adapters

`clockwork-pi` remains an executable reached through an existing action. It owns Pi profile validation, session identity, and Pi process launch. Clockwork does not gain a Pi dependency.

A future session-wake adapter must wait for a stable live-session API and a separate contract for identity, acknowledgement, expiry, redaction, and approval. Password injection remains a separate privileged adapter with explicit consent. Neither belongs in this refactor or in the generic invocation model.

## Call stacks and data flow

### Scheduled dispatch, proposed

```text
daemon tick timestamp
  -> dispatcher shell acquires dispatch lock
  -> state adapter loads JobState
  -> dispatcher shell observes any held per-job lock
  -> plan_dispatch(job, now, lock observation, proposed run ID)
  -> DispatchPlan
  -> state adapter saves changed job under state lock
  -> history adapter appends skipped-overlap records
  -> process adapter spawns detached _exec for Launch effects
```

Spawn failure follows the existing recovery path:

```text
_exec spawn error
  -> reload matching ScheduledClaim under state lock
  -> recover claim as internal error
  -> save job state
  -> append recovery RunRecord
  -> report safe dispatch error
```

### Scheduled execution, proposed

```text
raw _exec arguments
  -> commands::exec parser
  -> InvocationInput or InvocationInputError
  -> executor shell loads admitted job snapshot
  -> executor shell tries per-job lock
  -> decide_run(job, invocation, availability, observed_at)
  -> Start, Skip, or Ignore
  -> create log for Start
  -> action_runner executes existing Action
  -> ActionExit or ActionRunError
  -> classify_outcome
  -> reload current job under state lock
  -> complete_run
  -> save updated job state
  -> append RunRecord
  -> optionally write failure line and spawn _exec-fallback
```

### Manual execution, proposed

```text
clockwork run <job>
  -> resolve ID or name
  -> construct manual Invocation with one requested_at and run_id
  -> same executor admission path
  -> same action runner
  -> same completion path
  -> existing CLI projection
```

For this refactor, manual admission and schedule mutation preserve current behavior. The new `InvocationSource` makes that behavior explicit so a later product decision can change it in one pure function.

### Fallback, unchanged

```text
unsuccessful primary CompletionPlan
  -> executor shell emits FailureRequest
  -> append failures.log
  -> spawn detached _exec-fallback
  -> resolve per-job or global fallback command
  -> acquire fallback lock
  -> execute stripped-environment command
  -> append linked fallback RunRecord
```

Fallback does not call `decide_run` and does not target the primary action.

### Future local caller, deferred

```text
agent hook or local tool
  -> future CLI parser
  -> bounded external source label
  -> Invocation
  -> existing executor admission and completion path
```

The future parser is the extension point. Clockwork still executes the job's existing action. A source label never selects an adapter, grants authority, or carries secret content.

## Files to add, change, or delete

### Add

| File | Responsibility |
| --- | --- |
| `src/model/invocation.rs` | Internal primary invocation, source, attempt, admission, and input-error types. |
| `src/schedule/occurrence.rs` | Pure due and next-occurrence calculations shared by dispatch and output. |
| `src/engine/policy.rs` | Pure dispatch, admission, claim-recovery, outcome, and completion plans. |
| `src/engine/action_runner.rs` | Existing command, prompt, webhook, timeout, and process-control effects. |

### Change

| File | Change |
| --- | --- |
| `src/model/mod.rs` | Export `invocation`. |
| `src/model/job.rs` | Rename Rust type `InFlightRun` to `ScheduledClaim`; preserve serialized fields. |
| `src/model/run_record.rs` | Add narrow projections from invocation and outcome types; keep serialized values. |
| `src/manifest/plan.rs` | Update the Rust claim type name in existing reconciliation tests; preserve manifest behavior. |
| `src/schedule/mod.rs` | Export `occurrence`. |
| `src/engine/mod.rs` | Export `policy` and `action_runner`. |
| `src/engine/dispatcher.rs` | Retain I/O orchestration; replace inline due, overlap, and recovery policy with plans. |
| `src/engine/executor.rs` | Retain run orchestration and fallback shell; replace inline admission, completion, and action helpers. |
| `src/commands/run.rs` | Construct a manual `Invocation` and project `ExecutionDisposition`. |
| `src/commands/exec.rs` | Parse hidden command input into a typed `Invocation`. |
| `src/output/format.rs` | Use shared occurrence functions and one caller-supplied `now` per projection. |
| `tests/cli_dispatch.rs` | Preserve scheduled claim, overlap, recovery, one-shot, and daemon-detach behavior. |
| `tests/cli_run.rs` | Preserve manual execution, overlap, logs, and count behavior. |
| `tests/cli_fallback.rs` | Prove fallback remains follow-up behavior after completion extraction. |
| `docs/engine.md` | Document the functional core, imperative shells, and unchanged external boundary. |
| `docs/provenance/rust-parity.md` | Record `9f58aa0` as the exact source-parity baseline and define post-refactor behavior-parity evidence. |

### Delete

No files. Delete only the duplicated private helper functions after their callers move to the new modules.

## Red-green test plan

Each slice starts with one failing behavior test, adds the smallest contract and implementation, then reruns the existing CLI tests.

### Baseline: preserve parity evidence

1. Confirm HEAD is descended from the audited `9f58aa0` baseline.
2. Record the current 203 Rust and 16 Node passing-test baseline in `docs/provenance/rust-parity.md`.
3. State that source-tree parity applies to `9f58aa0`. State that later commits use stable serialized, CLI, runtime, and managed-integration behavior as parity evidence.
4. Do not list every moved function as a frozen-source exception. The refactor is an intentional architecture divergence.

### Slice 1: share occurrence calculations

1. Add fixed-time tests for recurring cron, recurring interval, one-shot, skipped occurrences, and an in-flight claim anchor.
2. Add `schedule::occurrence` and move only the calculation needed by those tests.
3. Make `dispatcher.rs` use the shared functions.
4. Make `output/format.rs` use the same functions and pass one `now` value.
5. Run `cargo test output::format engine::dispatcher schedule::occurrence` and `tests/cli_dispatch.rs`.

### Slice 2: type and decide primary invocation admission

1. Add pure tests for a matching scheduled claim, a stale claim, an inactive job, a free manual run, a busy manual run, and a busy scheduled run.
2. Add `model::invocation` and `policy::decide_run`.
3. Parse `_exec` and manual `run` inputs into `Invocation`.
4. Wire `executor.rs` to the decision while retaining the lock guard for `Start`.
5. Add a CLI behavior test that a concurrent manual run records one `skipped_overlap` result.

### Slice 3: complete runs through a pure transition

1. Add fixed-time tests for success, non-zero exit, timeout, internal error, claim clearing, run count, consecutive failures, and one-shot completion.
2. Add `classify_outcome` and `complete_run`.
3. Wire state update and history projection through `CompletionPlan`.
4. Prove that failure, timeout, and internal error produce a `FailureRequest`, while success does not.
5. Run `tests/cli_run.rs`, `tests/cli_history.rs`, and `tests/cli_fallback.rs`.

### Slice 4: plan due claims and recovery

1. Add pure tests for no due occurrence, consumed skip, new claim, overlap records for each missed occurrence, fresh claim retention, and stale claim recovery.
2. Add `plan_dispatch` and `recover_claim`.
3. Keep state lock, history append, and process spawn in `dispatcher.rs`.
4. Preserve the current state-before-history write order.
5. Run `tests/cli_dispatch.rs` and the repair tests.

### Slice 5: isolate action effects

1. Add a manual command test for exit and captured output.
2. Add a timeout test that proves the run becomes `timeout` and the process group stops.
3. Add a prompt test with a local fake agent executable.
4. Add a webhook test with a local HTTP server and `allow_insecure_http=true`. Do not make an external network call.
5. Move action-specific code to `action_runner.rs` one action kind at a time.
6. Keep executor outcome and persistence behavior unchanged after each move.

### Slice 6: protect fallback separation

1. Add a pure test that a fallback cannot become a primary `Invocation`.
2. Keep `_exec-fallback` and `Trigger::Fallback` on the existing linked-history path.
3. Rerun all fallback tests after executor extraction.
4. Record the stale fallback lock-file count as a separate defect. Do not hide it in the refactor.

### Final validation

Run:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
npm test
node --check install.mjs
node --check services/clockwork/clockwork-jobs.mjs
node --check services/clockwork/pi-launcher.mjs
node --check services/clockwork/self-email-once.mjs
bash -n install.sh
bash -n services/clockwork/service.sh
bash -n services/clockwork/launchd-run.sh
git diff --check
```

Also rerun the existing temporary-HOME installer and doctor checks. Do not install, start a service, reconcile a production job, or cause an external effect.

## Risks and open questions

- **Compatibility drift.** The largest risk is changing an odd existing behavior during extraction. Fixed-time policy tests and existing CLI tests must land before each move.
- **Stale parity claims.** The current provenance audit describes the present tree. Pin it to `9f58aa0` before the refactor and report post-refactor behavior evidence separately.
- **Split persistence.** A state save can succeed before history append fails. This spec keeps that boundary. A durable journal or single transactional store needs a separate design.
- **Manual policy.** Paused manual runs, one-time manual runs, and manual schedule advancement need a product decision. This refactor preserves current behavior and exposes it through `InvocationSource`.
- **Fallback concurrency.** Current code counts fallback lock-file names, not held locks. Track and fix this separately before relying on the configured concurrency limit.
- **Safe errors.** Moving action code can accidentally put action content in `InternalError`. Tests must assert that sensitive prompt, header, body, and credential values stay absent.
- **Future source labels.** Define label grammar, persistence, and audit policy only when a real non-schedule caller exists.
- **Session wake.** Target identity, live-session proof, acknowledgement, expiry, redaction, and approval remain unresolved. Do not model wake as a generic trigger.

Unresolved questions for later:

- What should `clockwork run` do when a job is paused or one-time?
- Should manual invocation advance the schedule anchor?
- When should the fallback concurrency defect be fixed?
