# Cut over from the current scheduler

This is a future procedure. Do not run it as part of building Clockwork.

The rule is simple: the current service stops before Clockwork can start. Clockwork applies every migrated job paused, then enables jobs only after a separate approval and schedule check.

## Prepare the agents change

Change the canonical agents repository in a separate reviewed commit. Do not edit its live generated copy by hand.

1. Remove the current scheduler service profile, helper links, plist template, templates, and skill registration.
2. Add an explicit Clockwork integration entry that calls the standalone Clockwork installer from its approved checkout.
3. Register `clockwork` as the skill name.
4. Link only `clockwork-jobs`, `clockwork-pi`, and `clockwork-self-email-once` from the Clockwork checkout.
5. Use the Clockwork plist label `com.iurysouza.clockwork`.
6. Keep the installer preview-only by default. `--apply` must not load launchd or reconcile jobs.
7. Keep the local Rust build pinned to engine commit `d1ff3991fbc0562dabb84296e95954a0b038eb0e` and Rust `1.85.0`. The Clockwork checkout records this in `docs/provenance/frozen-source.json`.
8. Add temporary-HOME tests that prove the new path, label, links, paused-first source contract, and no-launchd apply contract.

Validate and commit that agents change before applying it. Get separate approval before changing the live agents profile.

## Stage Clockwork without activating it

1. Build and test the committed Clockwork checkout.
2. In a temporary HOME, run `./install.sh`, `./install.sh --apply`, and `./install.sh --doctor`.
3. On the real machine, after approval, run `./install.sh --apply --install-deps`. This can create Clockwork directories, links, a plist, and a binary receipt. It must not bootstrap launchd or start a daemon.
4. Do not call `services/clockwork/service.sh start` yet.
5. Do not apply or enable Clockwork jobs yet.

The current service remains the only scheduler during this stage.

## Freeze the current service

This phase needs an explicit operator approval because it creates a scheduling gap.

1. Record the current service status, current job status, and active run evidence without printing job prompts, environment values, message bodies, webhook payloads, or credentials.
2. Pause any job that has an active external effect or an unresolved one-time delivery attempt.
3. Wait until no current scheduler run, Pi process, delivery guard lock, or provider operation is in flight.
4. Stop the current launchd service with its existing service command.
5. Verify that its launchd target is unloaded and that no current scheduler daemon remains.
6. Do not disable, delete, or overwrite the old plist, user job directory, user environment file, or generated state yet.

If the service cannot stop cleanly, stop here. Do not start Clockwork.

## Migrate sources and state

The new user source path is `~/.agents/clockwork/jobs.d/`. The new runtime path is `~/.local/state/clockwork/`.

1. Copy each old user job directory to the new source root. Do not move the original directory.
2. Rename each manifest file to `clockwork.yaml`.
3. Replace only scheduler product identifiers in the copied source. Keep job names, schedules, action content, and Pi profile values unchanged.
4. Rename prompt agent identities from `schedx-pi-<job>` to `clockwork-pi-<job>`.
5. Set every copied manifest to `paused: true`, including jobs that had been active. The first Clockwork apply must never activate a job.
6. Run `clockwork-jobs check` and `clockwork-jobs plan` for every copied job. Resolve collisions and validation failures before any apply.
7. Run `clockwork-jobs apply --confirm <job> --no-input` for each approved job. Do not use `--confirm all` until every plan is reviewed.

Do not copy the old engine's `jobs.json`, `config.json`, ownership map, locks, PID, manifests, logs, or daemon state into Clockwork. Those records contain old runtime identities and can cause duplicate or stale scheduling decisions. Clockwork creates fresh runtime objects from the paused source.

Preserve the old state directory unchanged as the historical record. Do not import Pi session JSONL files, logs, report backups, or generic history because they may contain prompts or other sensitive execution content.

Migrate delivery receipts only when the matching Clockwork action needs the receipt to prevent a duplicate delivery. Copy them as opaque owner-only files to the matching Clockwork receipt path after the old service stops and before the matching new job can run. Do not print their contents. A specialised one-time delivery guard must see the old delivered or ambiguous receipt before it accepts a new attempt.

## Start without double-running

1. Run `services/clockwork/service.sh doctor`.
2. Confirm that the old service stays unloaded and that there is no old scheduler daemon.
3. Start Clockwork with `services/clockwork/service.sh start` only after the checks pass.
4. Confirm that Clockwork reports one daemon and that every migrated job remains paused.
5. For each job, get a separate approval to change only `paused: true` to `paused: false`.
6. Run `clockwork-jobs check`, `plan`, and approved `apply` for that one job.
7. Confirm `status: active` and the expected non-null `next_run` before treating it as enabled.

For a one-time job, use a new job identity if its timestamp changed. Treat `status: active` with `next_run: null` and no run history as blocked. Pause it before its timestamp passes.

## Verify and retain rollback evidence

1. Compare the enabled Clockwork job list against the reviewed migration list.
2. Verify one scheduler record, one Pi session, and one sanitised delivery receipt for each delivery class that was intentionally enabled.
3. Keep the old source, state, plist, and service helper files untouched until the new service has passed its agreed observation period.
4. If rollback is required, stop Clockwork first, confirm no run or delivery is in flight, then restore the old service. Never run both services together.

## Unresolved questions

- Which current delivery receipts must move to prevent duplicate sends?
- What observation period is acceptable before the old installation is removed?
