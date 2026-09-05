# Schedules and run results

Clockwork separates the time a job is due from the time its action starts. A scheduler must be running to find due work. A saved or enabled job does not start a scheduler by itself.

## Schedule expressions

| Expression | Meaning |
| --- | --- |
| `0 9 * * MON-FRI` | Monday to Friday at 09:00 in the machine's local time. |
| `every 30m` | At minutes 0 and 30 of each hour. |
| `every 6h` | At 00:00, 06:00, 12:00, and 18:00 local time. |
| `every 10s` | A recurring interval measured in seconds. |
| `in 4h` | Once, four hours after the expression is parsed. |
| `30m` | Shorthand for `in 30m`. |
| `2027-03-01T09:00:00+01:00` | Once at an absolute time, if that time is still in the future. |

Cron expressions have five fields: minute, hour, day of month, month, and day of week. Clockwork uses the Rust `cron` crate, whose weekdays run from `1` for Sunday to `7` for Saturday. Prefer names such as `MON-FRI` to avoid numeric weekday differences between cron implementations.

`every Nm`, `every Nh`, and `every Nd` expand to cron expressions. They follow calendar boundaries rather than measuring elapsed time since the last run. For example, `every 3d` means midnight on days 1, 4, 7, and so on, restarting each month. It is not a fixed 72-hour interval.

Seconds use an elapsed-time interval instead. The parser accepts positive integer durations with `s`, `m`, `h`, or `d`. Recurring cron shorthand accepts 1 to 59 minutes, 1 to 23 hours, or 1 to 30 days.

The CLI converts one-time schedules to absolute timestamps when creating or updating a job. Enabling the job later does not move that time. If it has passed, update the schedule before enabling.

## When actions start

The foreground daemon checks for due jobs every 10 seconds by default. `clockwork daemon --interval <seconds>` changes that check interval. The built-in launchd and systemd timers check every 60 seconds.

These are dispatch intervals, not execution-time guarantees. A sleeping machine or stopped scheduler cannot start work at the requested time.

When the scheduler resumes, an idle enabled recurring job runs the latest due occurrence rather than replaying every missed occurrence. Runs that became due while the job was disabled are not queued for catch-up when you enable it.

## Overlapping runs

Clockwork claims a run before starting its executor. A claim reserves the job, but does not prove that the action process has started.

If a scheduled time arrives while that job's previous action is running, Clockwork records `skipped_overlap` instead of launching another action. Other jobs can run concurrently.

`clockwork job trigger` requires an enabled, idle job. It is an explicit immediate run, not a repair command for a missed schedule. `update` and `delete` also refuse while a run is in flight.

`disable` prevents future claims. It does not cancel an action already running.

## Read the result

Use `clockwork job history <name>` for results and `clockwork job logs <name>` for captured output. History includes the run ID, trigger, scheduled time, start and finish times, status, and exit code when available.

| Run status | Meaning |
| --- | --- |
| `success` | The command or agent exited with code 0, or the webhook returned a successful response. |
| `failed` | The action returned a failure, such as a non-zero exit code or a failed HTTP request. |
| `timeout` | The action exceeded its time limit. |
| `internal_error` | Clockwork could not start or finish execution normally, or recovered an abandoned claim. |
| `skipped_overlap` | Another run prevented this occurrence from starting. |

A one-time job becomes `completed` after an action succeeds, fails, or times out. Completion does not mean success. Check `last_run.status` or history before deciding what to do next.

Clockwork captures command and agent stdout and stderr in the run log. For a webhook, it logs the response or request error. Log content can contain sensitive output from the action, so review it before sharing.

A successful process exit does not prove that an email arrived or that a provider accepted a change. Actions that send messages, charge money, or update remote data need their own delivery checks and protection against duplicate effects.

## Failure commands

The global `on_failure` configuration can run a command after a failure, timeout, or internal error. This is a separate fallback action, not a retry of the original action. Clockwork records fallback runs in history with the failed run's ID.

Failure commands run with a restricted environment and receive failure details through `CLOCKWORK_*` variables. Review a fallback command's effects before configuring it. A fallback can have its own failure, so inspect its record as well as the original run.
