# Release Clockwork

Clockwork publishes prebuilt macOS archives through GitHub Releases. `cargo-dist` creates the archives and checksums. Release Please controls versions, tags, and release notes.

## Publish a release

1. Merge Conventional Commit changes into `main`.
2. Release Please opens or updates a release PR.
3. Review and merge the release PR.
4. Release Please creates the `vX.Y.Z` tag and publishes the GitHub Release.
5. The Release workflow checks out that tag, builds both macOS archives, and uploads the archives, checksums, source tarball, and `install.sh`.

The [Release workflow](../.github/workflows/release.yml) installs cargo-dist with Rust 1.96.1 and builds Clockwork with the Rust 1.85.0 toolchain pinned in `rust-toolchain.toml`.

Let Release Please create tags and GitHub Releases. It tracks the current version in [`.release-please-manifest.json`](../.release-please-manifest.json).

## Versioning

- `feat:` creates a minor release.
- `fix:` creates a patch release.
- `feat!:` or a breaking-change footer creates a minor release while Clockwork is below 1.0.
- `docs:`, `ci:`, `chore:`, and pure `refactor:` commits do not release.

## Check before merging

The [Verify workflow](../.github/workflows/verify.yml) must pass before you merge a release PR:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked --release
npm test
```

The release workflow also checks that each archive contains the binary and optional service files. The installer verifies the matching archive against `sha256.sum` before it writes the binary.
