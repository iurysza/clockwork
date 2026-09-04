# Release Clockwork

Clockwork publishes prebuilt macOS archives through GitHub Releases. `cargo-dist` creates the archives and checksums. Release Please controls versions, tags, and release notes.

## Normal releases

1. Merge Conventional Commit changes into `main`.
2. Release Please opens or updates a release PR.
3. Review and merge the release PR.
4. Release Please creates the `vX.Y.Z` tag and publishes the GitHub Release.
5. The Release workflow checks out that tag, builds both macOS archives, and uploads the archives, checksums, source tarball, and `install.sh`.

The Release workflow uses Rust 1.96.1 to run cargo-dist. Cargo-dist builds Clockwork with the repository's pinned Rust 1.85.0 toolchain.

Do not create tags or GitHub Releases manually after the bootstrap release.

## Bootstrap v0.1.0

The published `v0.1.0` tag establishes the initial release baseline. `.release-please-manifest.json` records that version, so future versions follow Conventional Commit messages without a `release-as` override.

## Versioning

- `feat:` creates a minor release.
- `fix:` creates a patch release.
- `feat!:` or a breaking-change footer creates a minor release while Clockwork is below 1.0.
- `docs:`, `ci:`, `chore:`, and pure `refactor:` commits do not release.

## Release checks

Before merging a release PR, Verify must pass:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked --release
npm test
```

The release workflow also checks that each archive contains the binary and optional service files. The installer verifies the matching archive against `sha256.sum` before it writes the binary.
