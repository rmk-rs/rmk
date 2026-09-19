# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Project Is

RMK is a Rust keyboard firmware library targeting embedded microcontrollers (`no_std` by default). It supports USB and BLE keyboards, split keyboards, on-the-fly keymap configuration(Vial), and advanced key behaviors (combos, tap-hold, macros, morse, etc). The repo is a Cargo monorepo with four crates():

- **`rmk`** — core firmware logic
- **`rmk-config`** — TOML configuration parsing (for `keyboard.toml`-based users)
- **`rmk-macro`** — procedural macros (`#[rmk_keyboard]`, `#[derive(Event)]`, etc.)
- **`rmk-types`** — shared types (`KeyAction`, `EncoderAction`, etc.)

Note that there's no `Cargo.toml` in the project root.

The `examples/` directory shows two usage patterns: `use_config/` (driven by `keyboard.toml`) and `use_rust/` (pure Rust API).
The `docs` directory contains multi-version documentation
The `scripts` directory contains useful scripts for checking/formatting code

## Commands

### Testing

Keyboard behavior is tested by the TOML scenarios in `rmk/tests/scenarios/`, which
`run_tests!` expands into ordinary tests. Read `rmk/tests/scenarios/README.md`
before adding a test: it documents the step and assertion vocabulary, and how to
write a case that fails for the right reason.

Dev loop — one feature set, from `rmk/` (requires `cargo nextest`):
```bash
cargo nextest run --no-default-features --features=split,vial,storage,async_matrix,_ble
```

Run macro tests from `rmk-macro/` (requires `cargo expand`).

Run format test from root:
```bash
bash scripts/format_all.sh
```

Check `Cargo.toml` dependency ordering from root (run without `--check` to fix):
```bash
bash scripts/sort_all.sh --check
```
Dependencies are sorted with `cargo sort --grouped`: the keys within each
blank-line-separated group are alphabetised and the groups stay in place.
In the examples, put `rmk` first, then the `esp-*`/`nrf-*`/`embassy-*`
dependencies, then the rest, each group separated by a blank line.

Behavioral test suite (the three RMK feature rows CI runs):
```bash
bash scripts/test_all.sh
```

### Building examples
Examples target specific MCUs; build from an example directory, e.g.:
```bash
cd examples/use_config/nrf52840_ble
cargo build --release
```

## Architecture

### Basic Data flow
```
Matrix / InputDevices → Events (pub/sub channels) -> InputProcessors/Keyboard(keyboard.rs) -> KEYBOARD_REPORT_CHANNEL -> HidWriter (USB or BLE) -> Host
```

### `keyboard.toml` and compile-time constants

`keyboard.toml` is parsed by `rmk-config` (`KeyboardTomlConfig`) at two points: by `rmk-types/build.rs` at build time, and by `rmk-macro` at macro-expansion time. The path defaults to `keyboard.toml` next to `Cargo.toml` and can be overridden with `KEYBOARD_TOML_PATH` in user space's `.cargo/config.toml`.

Config is loaded in three layers (later overrides earlier): `event_default.toml` → chip-specific default (from `rmk-config/src/default_config/<chip>.toml`, selected via `[keyboard].chip`) → user `keyboard.toml`.

`rmk-types/build.rs` reads only the `[rmk]` and `[event]` sections, then emits `constants.rs` as Rust `const` items. The full `KeyboardTomlConfig` struct in `rmk-config/src/lib.rs` is the authoritative reference for all available fields and their defaults.

`[event]` tunes per-event pub/sub channel sizes (`channel_size`, `pubs`, `subs`). All event names and their defaults live in `rmk-config/src/default_config/event_default.toml`.

## Rules

- Don't use `pub use` for convenient usage **within** the crate
- Don't add a small helper function (≤ 10 lines) that has only one call site — inline it at the call site
- When writing code comments, follow the principles in [Best practices for writing code comments](https://stackoverflow.blog/2021/12/23/best-practices-for-writing-code-comments/)
- When writing documentation and READMEs, follow the principles in [Best practices for GitHub Docs](https://docs.github.com/en/contributing/writing-for-github-docs/best-practices-for-github-docs)
- If you need a paragraph-long comment to justify why the workaround is OK, the code is wrong — fix the code.
