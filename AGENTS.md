# Repository Guidelines

RMK is a Rust keyboard firmware library targeting embedded microcontrollers (`no_std` by default). It supports USB and BLE keyboards, split keyboards, on-the-fly keymap configuration(Vial), and advanced key behaviors (combos, tap-hold, macros, morse, etc).

## Project Structure & Module Organization

The repo is a Cargo monorepo:

- `rmk/`: firmware runtime.
- `rmk-config/`: TOML parsing.
- `rmk-macro/`: procedural macros.
- `rmk-types/`: shared types.
- `rynk/`: separate workspace for host clients and transports.
- `examples/use_config/` and `examples/use_rust/`: board firmware using TOML configuration or Rust APIs.
- `rmk/tests/scenarios/` and `rmk/tests/integration/`: behavioral tests.
- `docs/`: versioned documentation and images.
- `scripts/` and `.github/ci/`: contain development automation.

Each crate has its own `Cargo.toml` and `src/`; there is no root `Cargo.toml`.

## Build, Test, and Development Commands

Use stable Rust and install `cargo-nextest`. Run scripts from the repository root; run Cargo commands from the indicated directory.

- `examples/use_config/nrf52840_ble/`: `cargo build --release` builds board firmware.
- `rmk/`: `cargo nextest run --no-default-features --features=split,vial,storage,async_matrix,_ble` runs a focused feature combination.
- `rmk-macro/`: `cargo nextest run --features _simulator` runs macro tests; install `cargo-expand` first.
- `bash scripts/test_all.sh` runs the RMK feature matrix, shared-type tests, and host-tooling checks.
- `bash scripts/format_all.sh` formats Rust code using nightly rustfmt.
- `bash .github/ci/check.sh` runs compile and Clippy checks across supported configurations.

## Coding Rules

- Keep things in one function unless composable or reusable
- Validate unknown values once at the boundary that owns them. Pass typed values inward instead of repeating type checks and property-existence checks. Do not defensively revalidate values already guaranteed by a schema, constructor, or internal type.
- Do not extract single-use helpers preemptively. Inline the logic at the call site unless the helper is reused, hides a genuinely complex boundary, or has a clear independent name that improves the caller.
- Before adding complexity for a speculative or vanishingly unlikely race or security edge case, explain the concrete failure mode, likelihood, and complexity cost to the user and get their buy-in. Do not silently expand scope for theoretical robustness.
- Don't use `pub use` for convenient usage **within** the crate
- When writing code comments, follow the principles in [Best practices for writing code comments](https://stackoverflow.blog/2021/12/23/best-practices-for-writing-code-comments/)
- When writing documentation and READMEs, follow the principles in [Best practices for GitHub Docs](https://docs.github.com/en/contributing/writing-for-github-docs/best-practices-for-github-docs)
- If you need a paragraph-long comment to justify why the workaround is OK, the code is wrong — fix the code.

## Testing Guidelines

- Read `rmk/tests/scenarios/README.md` before adding scenarios.
- TOML cases expand into ordinary Rust tests.
- Name cases after the behavior they verify.
- Reuse board fixtures.
- Confirm regressions fail without the fix.

Select feature coverage from `RMK_TEST_FEATURESETS` in `.github/ci/_lib.sh`. Distinguish simulator results from physical hardware validation.

## Commit & Pull Request Guidelines

Follow recent commit style: `fix(scope): description`, `feat(scope): description`, or `docs: description`. Keep commits focused, check existing issues/PRs, and link relevant issues. Limit PR descriptions to three concise lines explaining the change and purpose; don't report validation. Ensure required CI checks pass before merging.
