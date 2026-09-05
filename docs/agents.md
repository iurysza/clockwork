# Configure agent profiles

A profile tells Clockwork which agent binary to run, which arguments to pass, and how to send the prompt. Jobs reference profiles by name, so several jobs can share one configuration.

Install and authenticate the agent before scheduling it. Clockwork does not install agent binaries or manage their credentials.

## Detect installed agents

```sh
clockwork agent detect
clockwork agent list
```

Detection registers supported binaries found on `PATH` and saves their absolute paths. It preserves existing profiles unless you pass `--force`.

Review the arguments before enabling a job. Detected profiles use these defaults:

| Profile | Command before the prompt |
| --- | --- |
| `pi` | `pi --print --mode json` |
| `claude` | `claude -p --enable-auto-mode` |
| `codex` | `codex exec --full-auto` |
| `gemini` | `gemini -p --yolo` |
| `opencode` | `opencode run` |

Some defaults allow the agent to act without asking for tool approval. The installed agent controls what those flags permit. Check its help and permissions before leaving it unattended.

To select the profile used when a job omits `--profile`:

```sh
clockwork agent default pi
```

## Add a custom profile

From an existing project directory, register a Pi profile that reads prompts from stdin:

```sh
clockwork agent add pi-project \
	--bin "$(command -v pi)" \
	--cwd "$PWD" \
	--prompt-stdin \
	--arg=--print \
	--arg=--mode \
	--arg=json
```

`--arg` is repeatable. Use `--arg=--flag` when an argument starts with a dash. Clockwork passes these values to the agent unchanged.

Without `--prompt-stdin`, Clockwork appends the prompt as the final command-line argument. With it, Clockwork writes the prompt to stdin and closes the stream.

`agent add` replaces an existing profile with the same name. It does not have the job commands' preview-and-confirmation flow. Review changes to a shared profile because they affect every job that uses it.

### Working directories

A prompt job's `--cwd` overrides the profile's `--cwd`. Both accept a leading `~` and must resolve to an existing directory. If neither is set, the agent inherits the executor's working directory. Set one explicitly for project work.

Command jobs use `--workdir` instead. Use an absolute path there.

### Models, tools, and sessions

Use fixed `--arg` values for the model, tool restrictions, or session options supported by your agent. Check the installed agent's help for the exact flags.

If a job needs to reuse an agent session, give it a dedicated profile with that agent's session arguments. Clockwork does not create or resume sessions itself. Agent sessions are separate from Clockwork's run history.

## Check background execution

The macOS service starts through launchd rather than your interactive shell. It reads `~/.agents/clockwork/env` and sets its own `PATH`. An absolute agent path does not guarantee that tools launched by the agent are also available.

Before enabling a job, check the agent's credentials, project access, and required tools in that environment. See the [service environment reference](../services/clockwork/README.md#environment).

## Install Clockwork guidance for an agent

Preview where `clockwork setup` would install its bundled skill:

```sh
clockwork setup --dry-run
```

Then install it for detected agents:

```sh
clockwork setup
```

Setup also registers detected agent profiles. Use `--agent <name>` to select a skill destination, or `--all` to install for every supported destination. Selecting a destination does not limit profile detection.

Setup preserves existing skill files by default. `--force` replaces them and refreshes detected profiles, so review both before using it.

## Remove a profile

Inspect jobs that use the profile before removing it:

```sh
clockwork agent rm pi-project
```

Deleting a job does not delete its profile. Removing a profile does not delete jobs, but prompt jobs that still reference it cannot execute through that profile.
