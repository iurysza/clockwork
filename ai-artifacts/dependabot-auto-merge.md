# Dependabot auto-merge

Dependabot pull requests squash-merge after the Verify workflow's `verify` job succeeds. Semver-major updates stay open for a human.

The workflow is [`.github/workflows/dependabot-auto-merge.yml`](../.github/workflows/dependabot-auto-merge.yml). It runs on pull requests to `main` when `github.actor` is `dependabot[bot]`. [`dependabot/fetch-metadata`](https://github.com/dependabot/fetch-metadata) reads the update type. `version-update:semver-major` stops the job. Every other update type polls the `verify` check, which is the only CI job this repository runs on pull requests. After that check passes, the job runs `gh pr merge --auto --squash` for the head commit that started the run.

`gh pr merge --auto` by itself waits only for required status checks, and this repository has no ruleset that requires Verify. Polling `verify` first is the gate. Auto-merge after that pass still honors any other branch protection, and it does not deadlock on this job if the job itself is a required check. The command does not bypass branch protection.

There is no `dependabot.yml` yet. The workflow runs once Dependabot is configured and opens a pull request.
