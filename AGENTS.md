# Odyssey-rs Agent Framework Guidelines

> High-quality, concise, and actionable coding & contribution guidelines for the odyssey-rs agent framework.

---

## 1. Purpose

This document describes the coding standards, repository conventions, testing workflows, and contribution checklist for the **odyssey-rs** agent framework. It exists to keep the codebase readable, safe, and production-ready while enabling fast iteration.

## 2. Project & crate naming

* All crates in this repository must be prefixed with `odyssey-rs-`. Example: `odyssey-rs-core` for the `core` crate.

## 3. Style & idioms (Rust)

Follow these rules strictly to maintain consistency and quality across the project:

* **Inline `format!` arguments.** When you use `format!` and can embed variables into `{}`, always inline them: `format!("user: {}", user)`.
* **Collapse `if` statements.** Prefer the collapsible `if` style recommended by Clippy: avoid nested `if` that can be merged. See Clippy rule `collapsible_if`.
* **Use method references.** Prefer `iter.map(Type::method)` over `iter.map(|x| x.method())` when applicable. (Clippy: `redundant_closure_for_method_calls`.)
* **Exhaustive `match`.** When `match`-ing enums, make arms exhaustive. Prefer explicit arms over wildcard `_` unless truly appropriate.
* **Avoid `unwrap()` in library code.** Propagate errors with `?` and use contextual error types. Panics are allowed only in binaries for truly fatal states.
* **Small functions and clear names.** Keep function bodies small and focused. Prefer clarity over cleverness.
* **No placeholder or demo-only code** in core or shared crates. Each module must be fully functional.

## 4. Clippy, formatting and tooling

Automate and run these checks after making code changes.

* Run formatting automatically after code edits: `cargo fmt --all`.
* Run clippy and treat warnings as errors: `cargo clippy --workspace --all-targets -- -D warnings` and fix all issues.
* Use `tokei` to monitor Rust LOC frequently: `tokei -t Rust --exclude tests`.
* Install required system commands referenced by the repo before executing scripts (e.g., `rg`, `tokei`). If a CI script relies on a tool, ensure it is declared or installed in the development docs.

## 5. Tests & test conventions

Testing must be thorough, deterministic, and clear.

* **Run unit tests for changed crates.** If you change `odyssey-rs-core`, run `cargo test -p odyssey-rs-core`.
* **When common/core/protocol change:** after per-crate tests pass, run the full suite: `cargo test --all-features` (ask before running the full test suite if heavy compute/time is expected).
* **Assertions:** Prefer `pretty_assertions::assert_eq!` inside tests for readable diffs. Import this at the top of test modules.
* **Equality checks:** Prefer deep equality — assert whole objects rather than checking fields one-by-one.
* **Async tests:** Use `#[tokio::test]` where required.
* **Integration tests:** Place in `crates/<crate>/tests/` with descriptive snake_case filenames.
* **Avoid mutating process environment** in tests. Inject environment values where possible.
* coverage: `cargo install cargo-tarpaulin` then `cargo tarpaulin --all-features --out html`.
* **Test Coverage Percentage** should be above 70%, If new code changes are made ensure the coverage is above 70%.

## 6. API changes & docs

* Any change that adds or modifies a public API must update documentation under `docs/` accordingly. Keep docs consistent with code — broken or outdated docs are not acceptable.
* Update the changelog or release notes if the change affects users.

## 7. Error handling

* Prefer returning `Result<T, E>` with meaningful error types and context.
* Use `thiserror`, `anyhow` (in binaries), or well-structured domain error enums for libraries.
* Add contextual `map_err` or `with_context` messages where helpful for debugging.

## 8. Logging & diagnostics

* Use structured logging when the project depends on an external logging crate.
* Avoid `println!` in library code — reserve it for CLIs, examples, or scripts.

## 9. Code quality & maintainability

* Keep code **production-ready**: no half-baked implementations, TODOs that block functionality, or commented-out chunks left in the merge.
* Avoid speculative abstractions. Generalize only when there is a clear, repeated need.
* Prefer traits only when extension is expected.
* Provide clear comments for public APIs and complex internals. Comments should explain *why*, not *what*.

## 10. Naming conventions

* `snake_case` for functions, variables, modules.
* `CamelCase` for types, traits, enums.
* `SCREAMING_SNAKE_CASE` for constants.
* Use descriptive domain-focused names; avoid abbreviations.

## 11. Repository workflow & git

* Use small, focused commits with clear messages.
* Before creating a PR, run `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, and the relevant tests.
* Use `git` commands to inspect changes (e.g., `git diff`, `git show`, `git status`) to double-check the changes introduced across crates.

## 12. CI expectations

* CI will run `cargo fmt`, `cargo clippy` (as above), and the test suite.
* Ensure CI dependencies are declared and reproducible.

## 13. When editing AutoAgents or cross-project shared code

* **Always** ask the repository owner or the user before introducing changes that alter AutoAgents behavior or its public API.
* Do not implement insecure or non-performant workarounds without explicit approval.

## 14. Build & developer commands (cheat sheet)

* Format: `cargo fmt --all`
* Clippy: `cargo clippy --workspace --all-targets -- -D warnings`
* Test (single crate): `cargo test -p <crate-name>` e.g. `cargo test -p odyssey-rs-core`
* Full test suite (after per-crate tests pass): `cargo test --all-features`
* LOC check: `tokei -t Rust --exclude tests`
* Install `rg`, `tokei`, etc., if referenced by scripts before running them.

## 15. Quality checklist (this should be followed after changes)
* [ ] Code compiles and `cargo fmt --all` applied.
* [ ] `cargo clippy` completes with zero warnings.
* [ ] Relevant unit and integration tests pass locally for changed crates.
* [ ] Updated `docs/` for any API changes.
* [ ] No TODOs, commented-out code, or silent failures.
* [ ] Commits are small, descriptive, and logically grouped.
* [ ] If you updated any shared module (`common`, `core`, `protocol`), confirm full-suite tests were run and passed.

## 18. Docs & examples

* Provide minimal, copy-paste runnable examples for public APIs in doc comments (`///`).
* Keep `docs/` up-to-date with usage notes and any platform/tooling dependencies.
