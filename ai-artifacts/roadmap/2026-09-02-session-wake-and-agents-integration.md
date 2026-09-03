# Roadmap: session wake and agents integration

Status: proposed. Do not build this feature yet.

## Future capability: session wake

Clockwork may later schedule a one-time **session wake**. A wake is a deferred continuation request for an existing agent session. It is not a new agent process and it is not a generic scheduler action.

At the requested time, a runtime adapter can deliver a bounded message to the named live session. The session can resume work, run a script, or return a result into its original conversation.

Clockwork stays agent-neutral. It records the requested wake and its outcome. A runtime adapter owns live-session discovery, message delivery, and result return.

A wake must fail closed when the target session is unavailable. Do not restart a session or retain a wake for later delivery until a separate decision defines those semantics.

Do not treat privileged local actions, including password injection, as session wakes. They need a separate consented adapter and approval boundary.

## Milestone 1: finish feature parity

Before adding a wake feature:

- compare the complete engine tree against the frozen source after mechanical renames;
- retain only documented Clockwork differences;
- restore missing engine behavior before extracting new abstractions;
- keep the managed runtime defaults and safety contracts;
- preserve Rust, Node, shell, plist, temporary-HOME, legacy-name, secret, and diff checks.

## Milestone 2: agents integration compatibility

Make Clockwork consumable by the agents repository without making the agents repository own Clockwork.

The future agents change should:

- invoke the standalone Clockwork installer from a pinned local checkout;
- register the Clockwork skill;
- expose the managed helper commands;
- keep job source and generated state in Clockwork paths;
- preserve preview, paused-first, explicit apply, and separate service-start approval;
- add temporary-HOME tests for paths, links, plist label, and no-launchd apply.

The existing cutover plan names the later agents-repository edits and the zero-double-run migration order.

## Milestone 3: prove a second invocation source

Add a narrow invocation command only when a real coding-agent hook needs it. The command should request execution of an existing job and record the source. It should share the scheduler's lock, overlap, history, and result rules.

Do not add a plugin framework before a second real adapter exists.

## Milestone 4: evaluate session wake

Build a runtime adapter only after the target agent runtime provides a stable live-session control API. Define target identity, delivery acknowledgement, expiry, failure result, audit data, redaction, and user approval before implementation.

## Open decision

The current safety boundary says the agents repository is read-only. Clarify whether the next step is only compatibility work in Clockwork or an approved change to the agents repository.
