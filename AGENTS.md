# Agent map

Clockwork schedules agent prompts, local commands, and HTTPS webhooks.

## Tree

The paths that matter for this loop are:

- `src/` for the Rust CLI
- `install.sh` and `install.mjs` for the install scripts
- `install.test.mjs` and `install-release.test.mjs` for the tests that `npm test` runs
- `.github/workflows/verify.yml` for the CI commands below
- [`README.md`](README.md) and [`docs/`](docs/) for human product docs

## ai-artifacts

Keep how-it-works notes in `ai-artifacts/` current. Start at [`ai-artifacts/_index.md`](ai-artifacts/_index.md). Add architecture notes under `ai-artifacts/` as you learn the system.

## Closed loop

1. Research the change against the tree and the docs.
2. Change the code.
3. Run the verify commands in this order:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked --release
npm test
node --check install.mjs
bash -n install.sh
```

## Git commits

Never include Cursor (or any Cursor agent/bot) as git author, committer, or in a Co-authored-by / similar trailer.
