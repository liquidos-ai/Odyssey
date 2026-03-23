# Cargo Publish Guide for Odyssey

**Follow the below instructions in sequence**

1. Create a release branch from `main`:

```shell
git checkout main
git pull origin main
git checkout -b feature/vx.x.x
```

2. Update the workspace version in `Cargo.toml`.
   Update:
   - `[workspace.package].version`
   - every Odyssey crate version in `[workspace.dependencies]`

   We use SemVer versions.

3. Update release-facing docs for the new version if needed.
   If the release changes a public API, update the relevant docs under `docs/`.

4. Run the release checks before opening the PR:

```shell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --all-features
```

5. Commit the release changes on the release branch:

```shell
git add .
git commit -m "[MAINT]: bump version to x.x.x"
git push origin feature/vx.x.x
```

6. Open a PR from `feature/vx.x.x` to `main`, wait for CI to pass, and get it reviewed.

7. Merge the PR to `main`.

8. After the PR is merged, switch back to `main` and pull the merged commit that will be released:

```shell
git checkout main
git pull origin main
```

9. Make sure you are authenticated with crates.io:

```shell
cargo login
```

10. Publish to crates.io from `main` and MAINTAIN the order below.
    Run `cargo publish --dry-run` before each real publish.

```shell
cd crates/odyssey-rs-protocol
cargo publish --dry-run
cargo publish
```

```shell
cd ../odyssey-rs-manifest
cargo publish --dry-run
cargo publish
```

```shell
cd ../odyssey-rs-sandbox
cargo publish --dry-run
cargo publish
```

```shell
cd ../odyssey-rs-bundle
cargo publish --dry-run
cargo publish
```

```shell
cd ../odyssey-rs-tools
cargo publish --dry-run
cargo publish
```

```shell
cd ../odyssey-rs-runtime
cargo publish --dry-run
cargo publish
```

```shell
cd ../odyssey-rs-server
cargo publish --dry-run
cargo publish
```

```shell
cd ../odyssey-rs-tui
cargo publish --dry-run
cargo publish
```

```shell
cd ../odyssey-rs
cargo publish --dry-run
cargo publish
```

11. Wait for crates.io to index each crate before publishing the next dependent crate.
    If a dependent publish fails because the previous crate version is not visible yet, wait briefly and retry.

12. Create the release tag on the merged `main` commit:

```shell
cd ../..
git tag -a vx.x.x -m "Release vx.x.x

Features:
-

Improvements:
-
"
```

```shell
git push origin vx.x.x
```
