//! `run_tests!`: expand a directory of simulator scenarios into `#[test]` fns.
//!
//! A scenario TOML holds a keyboard definition (a `keyboard.toml`, optionally
//! deep-merged over a referenced base board) and an array of named `[[test]]`
//! cases, each a `steps` timeline of physical input plus the `expect` list of
//! HID reports that input must produce, in order. The keyboard half
//! deserializes into the ordinary [`KeyboardTomlConfig`], so a real keyboard's
//! `keyboard.toml` works as a board as-is — simulation reads only its keymap
//! and behavior, and never resolves its hardware.
//!
//! Each file becomes a `mod <file_stem>` of tests driving the `SimKeyboard`
//! harness in rmk's `tests/integration/simulator`, expanding to what a
//! hand-written test would contain.

use std::fs;
use std::path::{Path, PathBuf};

use crate::codegen::action_parser::get_key_with_alias;
use crate::codegen::behavior::expand_behavior_config;
use crate::codegen::keymap::{expand_encoder_layer, expand_layer};
use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use rmk_config::KeyboardTomlConfig;
use rmk_config::resolved::{Behavior, Host, Keymap};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use syn::LitStr;
use toml::{Table, Value};

/// Milliseconds inserted before a step that no explicit `delay` already spaces
/// out. Well under every behavior timeout, so it orders steps without
/// resolving one.
const STEP_GAP_MS: u64 = 10;

/// `run_tests!("tests/scenarios")`, with the directory relative to `Cargo.toml`:
/// every `*.toml` directly in it is a scenario, so registering one is dropping
/// the file in. Boards live in a subdirectory and are read through the
/// `keyboard = "..."` reference instead.
pub(crate) fn expand_run_tests(dir: LitStr) -> TokenStream2 {
    expand_dir(&dir.value()).unwrap_or_else(|e| panic!("\n❌ {e}"))
}

fn expand_dir(relative: &str) -> Result<TokenStream2, String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let dir = Path::new(&manifest_dir).join(relative);
    let listing = fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in listing {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return Err(format!("no scenario files in {}", dir.display()));
    }
    // Sorted, so the expansion order is the order `nextest list` prints.
    paths.sort();

    let mods = paths
        .iter()
        .map(|path| expand_file(path).map_err(|e| format!("{}: {e}", path.display())))
        .collect::<Result<Vec<_>, _>>()?;

    // `expand_file` has rustc track every file this macro reads, but nothing
    // tracks the listing: a scenario added later changes no tracked file, so
    // cargo keeps the test binary that predates it. A deleted one leaves a
    // tracked path that no longer exists, which does rebuild — so only
    // additions can go unnoticed, and this catches them at run time.
    let names = paths.iter().map(|p| {
        let name = p.file_name().and_then(|n| n.to_str());
        name.expect("scenario paths are UTF-8 and name a file")
    });
    Ok(quote! {
        #(#mods)*

        #[test]
        fn scenarios_are_registered() {
            let expanded = [#(#names),*];
            let dir = ::std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(#relative);
            let listing = ::std::fs::read_dir(&dir).expect("scenario directory");
            let added: Vec<String> = listing
                .map(|e| e.expect("scenario entry").file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".toml") && !expanded.contains(&name.as_str()))
                .collect();
            assert!(
                added.is_empty(),
                "{added:?} reached this directory after the test binary was built; \
                 `touch {}` to expand them too",
                file!(),
            );
        }
    })
}

/// Read one scenario and its base board, then expand the whole file.
fn expand_file(path: &Path) -> Result<TokenStream2, String> {
    let doc = fs::read_to_string(path).map_err(|e| format!("cannot read: {e}"))?;
    let mut doc: Table = toml::from_str(&doc).map_err(|e| format!("scenario TOML: {e}"))?;

    // `keyboard = "..."` names the base board, relative to the scenario file.
    let base_path = match doc.remove("keyboard") {
        None => None,
        Some(Value::String(rel)) => Some(path.parent().expect("scenario has a parent").join(rel)),
        Some(_) => return Err("`keyboard` must be a path string".to_string()),
    };
    let base = base_path
        .as_deref()
        .map(|base| {
            fs::read_to_string(base).map_err(|e| format!("cannot read {}: {e}", base.display()))
        })
        .transpose()?;

    let tests = expand_scenario(doc, base.as_deref())?;
    let stem = path.file_stem().and_then(|s| s.to_str());
    let stem = stem.expect("scenario paths are UTF-8 and name a file");
    let mod_name = format_ident!("{}", stem.replace('-', "_"));

    // Editing a scenario has to re-expand it, and cargo only rebuilds on files
    // rustc reports as read. `include_bytes!` is that report; the constant
    // itself is never used.
    let tracked = [Some(path), base_path.as_deref()].into_iter().flatten();
    let tracked = tracked.map(|path| {
        let path = path.to_str().expect("scenario paths are UTF-8");
        quote! { const _: &[u8] = include_bytes!(#path); }
    });

    Ok(quote! {
        mod #mod_name {
            #(#tracked)*
            #(#tests)*
        }
    })
}

/// Expand every `[[test]]` of a scenario whose `keyboard = "path"` reference is
/// already resolved into `base`.
fn expand_scenario(mut doc: Table, base: Option<&str>) -> Result<Vec<TokenStream2>, String> {
    let raw_tests = match doc.remove("test") {
        Some(Value::Array(tests)) if !tests.is_empty() => tests,
        _ => return Err("scenario TOML: no [[test]] defined".to_string()),
    };
    // `[rmk]` capacities are compile-time constants baked into the test binary by
    // `rmk-types/build.rs`, so a scenario's would silently do nothing.
    if doc.contains_key("rmk") {
        return Err("scenario TOML: [rmk] capacities are compile-time constants and cannot take effect in sim tests".to_string());
    }
    let file_features: Vec<String> = match doc.remove("features") {
        None => Vec::new(),
        Some(value) => arg(&value, "features", "an array of cargo feature names")?,
    };

    let mut keyboard = match base {
        Some(base) => toml::from_str(base).map_err(|e| format!("base keyboard TOML: {e}"))?,
        None => Table::new(),
    };
    deep_merge(&mut keyboard, doc);

    // Duplicate test names need no check here: they collide as fn names in the
    // generated mod, which rustc reports.
    let raw_tests = raw_tests.into_iter().enumerate();
    raw_tests
        .map(|(i, raw)| expand_test(raw, i, &keyboard, &file_features))
        .collect()
}

/// Merge `over` into `base`: tables merge recursively, everything else
/// (arrays included) replaces wholesale.
fn deep_merge(base: &mut Table, over: Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(b)), Value::Table(o)) => deep_merge(b, o),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Expand one `[[test]]` into a `#[test]` fn. `file_features` are the scenario's
/// own `features`, which every test in it inherits.
fn expand_test(
    value: Value,
    index: usize,
    keyboard: &Table,
    file_features: &[String],
) -> Result<TokenStream2, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Test {
        name: String,
        steps: Vec<Value>,
        #[serde(default)]
        expect: Vec<Value>,
        behavior: Option<Value>,
        #[serde(default)]
        features: Vec<String>,
    }
    let test: Result<Test, _> = value.try_into();
    let test = test.map_err(|e| format!("[[test]] #{index}: {e}"))?;
    let ctx = format!("test '{}'", test.name);
    if test.steps.is_empty() {
        return Err(format!("{ctx}: `steps` must not be empty"));
    }

    let mut config_table = keyboard.clone();
    let delta = test.behavior.map(|d| ("behavior".to_string(), d));
    deep_merge(&mut config_table, delta.into_iter().collect());
    let config: Result<KeyboardTomlConfig, _> = config_table.try_into();
    let config = config.map_err(|e| format!("{ctx}: keyboard config: {e}"))?;
    let keymap = config.keymap().map_err(|e| format!("{ctx}: {e}"))?;
    let behavior = config.behavior().map_err(|e| format!("{ctx}: {e}"))?;

    // The input runs first and the assertions follow it, so a test reads as
    // "this happened, then the host saw that". Reports queue up meanwhile, so
    // splitting the two only moves when they are asserted, not what arrives.
    let mut steps = Vec::new();
    let mut timed = false;
    for (i, raw) in test.steps.iter().enumerate() {
        let delay = raw
            .as_table()
            .is_some_and(|step| step.contains_key("delay"));
        // Nothing advances the clock on its own, so steps would otherwise pile
        // up on one instant. A `delay` states the interval; every other step
        // gets STEP_GAP_MS, which keeps the delays a scenario spells out down to
        // the ones its behavior actually turns on.
        if !delay && !timed {
            steps.push(quote! { .delay(#STEP_GAP_MS) });
        }
        timed = delay;
        steps.push(input_step(&keymap, raw).map_err(|e| format!("{ctx}, steps[{i}]: {e}"))?);
    }
    for (i, raw) in test.expect.iter().enumerate() {
        steps.push(expectation(raw).map_err(|e| format!("{ctx}, expect[{i}]: {e}"))?);
    }

    let behavior_stmt = expand_behavior_config(&behavior);
    let features: Vec<&String> = file_features.iter().chain(&test.features).collect();
    // `[host]`'s lock gate and the layout blob are rynk types, so only a rynk
    // scenario can name them. Others keep the default `RmkConfig`, which is what
    // a `[host]`-less board gets anyway.
    let rmk_config = features
        .iter()
        .any(|f| f.as_str() == "rynk")
        .then(|| {
            let layout = config.layout().map_err(|e| format!("{ctx}: {e}"))?;
            Ok::<_, String>(expand_rmk_config(&config.host(), &layout.blob))
        })
        .transpose()?;
    let builder = expand_builder(&keymap, &behavior, rmk_config);
    let fn_name = format_ident!("{}", test.name);

    Ok(quote! {
        #[cfg(all(#(feature = #features),*))]
        #[test]
        fn #fn_name() {
            ::rmk::test_support::test_block_on(async {
                #behavior_stmt
                let mut keyboard = #builder .build().await;
                keyboard #(#steps)* .run().await;
            });
        }
    })
}

/// The `SimKeyboard::builder(..)` chain for one test: keymap layers, encoder
/// layers, and per-key handedness. `.build()` is left to the caller.
fn expand_builder(
    keymap: &Keymap,
    behavior: &Behavior,
    rmk_config: Option<TokenStream2>,
) -> TokenStream2 {
    let profiles = behavior.morse.as_ref().map(|m| m.profiles.clone());
    let sticky_profiles = behavior
        .sticky_key
        .as_ref()
        .map(|sticky| sticky.profiles.clone());
    let rows = keymap.rows as usize;
    let cols = keymap.cols as usize;
    let layers = keymap.layers as usize;
    let layer_tokens = keymap.keymap.iter();
    let layer_tokens = layer_tokens.map(|l| expand_layer(l.clone(), &profiles, &sticky_profiles));

    let num_encoder = keymap.num_encoder;
    let encoder_call = (num_encoder > 0).then(|| {
        // `expand_encoder_layer` pads a layer that lists no `encoders` with `No`.
        let mut encoder_map = keymap.encoder_map.clone();
        encoder_map.resize(layers, Vec::new());
        let encoder_layers = encoder_map
            .into_iter()
            .map(|e| expand_encoder_layer(e, num_encoder, &profiles, &sticky_profiles));
        quote! { .encoders([#(#encoder_layers),*]) }
    });

    let hand_rows = (0..rows).map(|row| {
        let hands = (0..cols).map(|col| match keymap.key_info[row][col].hand {
            'l' | 'L' => quote! { ::rmk::config::Hand::Left },
            'r' | 'R' => quote! { ::rmk::config::Hand::Right },
            '*' => quote! { ::rmk::config::Hand::Bilateral },
            _ => quote! { ::rmk::config::Hand::Unknown },
        });
        quote! { [#(#hands),*] }
    });
    quote! {
        crate::simulator::SimKeyboard::builder::<#rows, #cols, #layers>([#(#layer_tokens),*])
            .behavior_config(behavior_config)
            #encoder_call
            #rmk_config
            .hands([#(#hand_rows),*])
    }
}

/// The rynk half of `RmkConfig`: `[host]`'s lock gate, whose unlock keys are
/// `(row, col)` pairs the scenario can then press, plus the compressed layout
/// blob `GetLayout` pages out.
fn expand_rmk_config(host: &Host, layout_blob: &[u8]) -> TokenStream2 {
    let keys = host.unlock_keys.iter().map(|k| {
        let (row, col) = (k[0], k[1]);
        quote! { (#row, #col) }
    });
    let (insecure, write_requires_unlock) = (host.insecure, host.write_requires_unlock);
    let blob = proc_macro2::Literal::byte_string(layout_blob);
    quote! {
        .rmk_config(::rmk::config::RmkConfig {
            lock_config: ::rmk::config::LockConfig {
                unlock_keys: &[#(#keys),*],
                insecure: #insecure,
                write_requires_unlock: #write_requires_unlock,
            },
            layout_blob: #blob,
            ..Default::default()
        })
    }
}

/// One input step as a harness call appended to the keyboard's timeline.
/// Positions and encoder ids are bounds-checked here because the runtime keymap
/// indexes a flat array — an out-of-range one silently reads a neighbouring key
/// or layer.
fn input_step(keymap: &Keymap, value: &Value) -> Result<TokenStream2, String> {
    let one_op = value.as_table().filter(|t| t.len() == 1);
    let Some((op, v)) = one_op.and_then(|t| t.iter().next()) else {
        return Err("a step must be a table with exactly one key".to_string());
    };
    Ok(match op.as_str() {
        // `press`/`release` and `rotary_*` name their harness method directly.
        "press" | "release" => {
            let (row, col): (u8, u8) = arg(v, op, "[row, col]")?;
            check_key(keymap, row, col)?;
            let method = format_ident!("{op}");
            quote! { .#method(#row, #col) }
        }
        // Named fields, unlike `press`/`release`: a bare `[row, col, ms]` reads
        // as if the hold time were a third coordinate.
        "tap" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Tap {
                pos: (u8, u8),
                duration: u64,
            }
            let Tap {
                pos: (row, col),
                duration,
            } = arg(v, op, "{ pos = [row, col], duration = ms }")?;
            check_key(keymap, row, col)?;
            quote! { .tap(#row, #col, #duration) }
        }
        "delay" => {
            let ms: u64 = arg(v, op, "milliseconds (integer)")?;
            quote! { .delay(#ms) }
        }
        // Like `delay`, but the silence is the claim. It belongs among the steps
        // rather than in `expect`, because an expectation deferred to the end of
        // the timeline can no longer say *when* nothing was reported.
        "no_report" => {
            let ms: u64 = arg(v, op, "milliseconds (integer)")?;
            quote! { .expect_no_report(#ms) }
        }
        "rotary_cw" | "rotary_ccw" => {
            let id: u8 = arg(v, op, "an encoder id (integer)")?;
            let encoders = keymap.num_encoder;
            if id as usize >= encoders {
                return Err(format!("encoder {id} is outside the {encoders} declared"));
            }
            let method = format_ident!("{op}");
            quote! { .#method(#id) }
        }
        // Passkey entry brackets a stretch of the timeline: while it is open the
        // keys feed the passkey instead of the host, so nothing is reported.
        "passkey" => match arg::<String>(v, op, "\"begin\" or \"end\"")?.as_str() {
            "begin" => quote! { .begin_passkey_entry() },
            "end" => quote! { .end_passkey_entry() },
            other => return Err(format!("unknown `passkey` value \"{other}\"")),
        },
        "rynk" => rynk_step(v)?,
        // `rynk_topic` asserts the pushed frame; `rynk_publish` causes it by
        // publishing the internal event behind it. Both name their harness
        // method directly.
        "rynk_topic" | "rynk_publish" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct RynkTopic {
                topic: String,
                payload: Option<Value>,
            }
            let step: RynkTopic = arg(v, op, "{ topic = \"...\", payload = ... }")?;
            let topic = command_ident(&step.topic)?;
            let payload = json(step.payload.as_ref())?;
            let method = format_ident!("{op}");
            quote! { .#method(::rmk::types::protocol::rynk::Cmd::#topic, #payload) }
        }
        // Bytes straight onto the link, framed by nothing: the only way to state
        // what a malformed or oversized frame does to a session. `rynk_reply`
        // is its mirror, for replies to commands no endpoint can name.
        "rynk_raw" => {
            let bytes: Vec<u8> = arg(v, op, "an array of bytes")?;
            quote! { .host_send([#(#bytes),*]) }
        }
        "rynk_reply" => {
            let bytes: Vec<u8> = arg(v, op, "an array of bytes")?;
            quote! { .expect_host_frame([#(#bytes),*]) }
        }
        "rynk_no_reply" => {
            let ms: u64 = arg(v, op, "milliseconds (integer)")?;
            quote! { .expect_no_host_reply(#ms) }
        }
        other => return Err(format!("unknown step op `{other}`")),
    })
}

/// One expectation as a harness call appended to the timeline. A keyboard
/// report is the bare array of what is down; every other HID page names itself
/// in a table.
fn expectation(value: &Value) -> Result<TokenStream2, String> {
    let table = match value {
        Value::Array(_) => return keyboard_report(value),
        Value::Table(t) if t.len() == 1 => t,
        _ => {
            return Err(
                "an expectation is a keycode array or a table with exactly one key".to_string(),
            );
        }
    };
    let (op, v) = table
        .iter()
        .next()
        .expect("the table holds exactly one key");
    Ok(match op.as_str() {
        "mouse" => {
            #[derive(Deserialize, Default)]
            #[serde(deny_unknown_fields, default)]
            struct Mouse {
                buttons: u8,
                x: i8,
                y: i8,
                wheel: i8,
                pan: i8,
            }
            let m: Mouse = arg(v, op, "{ buttons, x, y, wheel, pan }")?;
            let (buttons, x, y, wheel, pan) = (m.buttons, m.x, m.y, m.wheel, m.pan);
            quote! {
                .expect_report(::rmk::hid::Report::MouseReport(::usbd_hid::descriptor::MouseReport {
                    buttons: #buttons,
                    x: #x,
                    y: #y,
                    wheel: #wheel,
                    pan: #pan,
                }))
            }
        }
        // The consumer/system pages carry one usage at a time, named in the same
        // keycode vocabulary the keymap uses; the empty array is the release.
        "consumer" => {
            let usage_id = match usage_key(v, op)? {
                None => quote! { 0 },
                Some(key) => quote! {
                    u16::from(
                        ::rmk::types::keycode::HidKeyCode::#key
                            .process_as_consumer()
                            .expect("`consumer` needs a consumer key"),
                    )
                },
            };
            quote! {
                .expect_report(::rmk::hid::Report::MediaKeyboardReport(
                    ::usbd_hid::descriptor::MediaKeyboardReport { usage_id: #usage_id },
                ))
            }
        }
        "system" => {
            let usage_id = match usage_key(v, op)? {
                None => quote! { 0 },
                Some(key) => quote! {
                    ::rmk::types::keycode::HidKeyCode::#key
                        .process_as_system_control()
                        .expect("`system` needs a system control key") as u8
                },
            };
            quote! {
                .expect_report(::rmk::hid::Report::SystemControlReport(
                    ::usbd_hid::descriptor::SystemControlReport { usage_id: #usage_id },
                ))
            }
        }
        "steno" => {
            let keys: Vec<String> = arg(v, op, "an array of steno key names")?;
            // A `StenoKey` is a bit index into the 64-bit chord bitmap. The array
            // is annotated so an empty chord still infers.
            let len = keys.len();
            let keys = keys.iter().map(|k| format_ident!("{}", k.to_uppercase()));
            quote! {
                .expect_report(::rmk::hid::Report::StenoReport({
                    let chord: [::rmk::types::steno::StenoKey; #len] =
                        [#(::rmk::types::steno::StenoKey::#keys),*];
                    let mut keys = [0u8; 8];
                    for key in chord {
                        keys[(key.0 / 8) as usize] |= 0x80 >> (key.0 % 8);
                    }
                    ::rmk::hid::StenoReport { keys }
                }))
            }
        }
        "passkey" => match v {
            Value::String(s) if s == "cancelled" => quote! { .expect_passkey_response(None) },
            Value::Integer(_) => {
                let passkey: u32 = arg(v, op, "a passkey or \"cancelled\"")?;
                quote! { .expect_passkey_response(Some(#passkey)) }
            }
            _ => return Err(format!("`{op}` must be a passkey or \"cancelled\"")),
        },
        other => return Err(format!("unknown expectation `{other}`")),
    })
}

/// One Rynk request as a `SimKeyboard::rynk` call.
///
/// Payloads pass through as JSON rather than being spelled per endpoint: the
/// request type's own serde shape is the schema, so a new endpoint needs no
/// change here and a wrong field is a deserialize error naming it. The command
/// is checked only as an identifier — an unknown one resolves to no type in
/// `command`, which rustc reports against the generated call.
fn rynk_step(value: &Value) -> Result<TokenStream2, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Rynk {
        cmd: String,
        payload: Option<Value>,
        reply: Option<Value>,
        error: Option<String>,
    }
    let step: Rynk = arg(
        value,
        "rynk",
        "{ cmd = \"...\", payload = ..., reply = ... }",
    )?;
    let cmd = command_ident(&step.cmd)?;
    let request = json(step.payload.as_ref())?;

    let reply = match (&step.reply, &step.error) {
        (Some(_), Some(_)) => {
            return Err("`reply` and `error` state the same thing; use one".to_string());
        }
        (_, Some(error)) => {
            let variant = command_ident(error)?;
            quote! {
                crate::rynk::RynkReply::Err(
                    ::rmk::types::protocol::rynk::RynkError::#variant,
                )
            }
        }
        // No `reply` means the unit response every setter returns, which is
        // JSON `null` — the same token an explicit `reply` of nothing produces.
        (reply, _) => {
            let response = json(reply.as_ref())?;
            quote! { crate::rynk::RynkReply::Ok(#response) }
        }
    };
    Ok(quote! {
        .rynk::<::rmk::types::protocol::rynk::command::#cmd>(#request, #reply)
    })
}

/// A protocol name as an identifier. `format_ident!` would panic on anything
/// else, and the panic names the macro rather than the scenario.
fn command_ident(name: &str) -> Result<Ident, String> {
    let ident = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ident {
        return Err(format!("`{name}` is not a protocol name"));
    }
    Ok(format_ident!("{name}"))
}

/// A scenario payload as the JSON the test deserializes. JSON rather than bytes
/// because the encoding must happen against the firmware's own `rmk-types`:
/// `Action` has a `#[cfg(feature = "steno")]` variant, so postcard discriminants
/// depend on a feature set this macro cannot see.
fn json(value: Option<&Value>) -> Result<String, String> {
    let Some(value) = value else {
        return Ok("null".to_string());
    };
    serde_json::to_string(value).map_err(|e| format!("payload is not expressible as JSON: {e}"))
}

/// The key named by a single-usage report expectation, or `None` when the array
/// is empty (the all-released report both pages send on key up).
fn usage_key(value: &Value, op: &str) -> Result<Option<Ident>, String> {
    let mut names: Vec<String> = arg(value, op, "an array of at most one key name")?;
    match names.len() {
        0 => Ok(None),
        1 => Ok(Some(get_key_with_alias(names.remove(0)))),
        n => Err(format!("`{op}` reports one key at a time, got {n}")),
    }
}

fn check_key(keymap: &Keymap, row: u8, col: u8) -> Result<(), String> {
    let (rows, cols) = (keymap.rows, keymap.cols);
    if row >= rows || col >= cols {
        return Err(format!(
            "position ({row}, {col}) is outside the {rows}x{cols} matrix"
        ));
    }
    Ok(())
}

/// Deserialize a step's argument, naming the shape it should have on failure.
fn arg<T: DeserializeOwned>(value: &Value, op: &str, shape: &str) -> Result<T, String> {
    let parsed: Result<T, _> = value.clone().try_into();
    parsed.map_err(|e| format!("`{op}` must be {shape}: {e}"))
}

/// A keyboard-report assertion containing modifiers and keycodes in any order.
fn keyboard_report(value: &Value) -> Result<TokenStream2, String> {
    const MODIFIERS: [&str; 8] = [
        "LCtrl", "LShift", "LAlt", "LGui", "RCtrl", "RShift", "RAlt", "RGui",
    ];
    let names = arg::<Vec<String>>(value, "expect", "an array of modifier/keycode names")?;
    let mut mods = 0u8;
    let mut keys = Vec::new();
    for name in names {
        if let Some(bit) = MODIFIERS.iter().position(|modifier| *modifier == name) {
            mods |= 1 << bit;
        } else {
            keys.push(name);
        }
    }
    let keycodes = keys.iter().map(|key| {
        let ident = get_key_with_alias(key.clone());
        quote! { ::rmk::types::keycode::HidKeyCode::#ident }
    });
    Ok(quote! { .expect_keys_with_mods(#mods, [#(#keycodes),*]) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
[layout]
rows = 1
cols = 1
map = "(0,0)"

[[keymap.layer]]
keys = "A"

[[test]]
name = "t"
steps = [{ press = [0, 0] }, { release = [0, 0] }]
expect = [[]]
"#;

    /// Expand a scenario the way `expand_file` does, minus the file reads.
    fn expand(doc: &str, base: Option<&str>) -> Result<String, String> {
        let doc: Table = toml::from_str(doc).expect("fixture is valid TOML");
        let tests = expand_scenario(doc, base)?;
        Ok(quote! { #(#tests)* }.to_string().replace(' ', ""))
    }

    /// Also pins the implicit gap: it precedes each step, and an explicit
    /// `delay` replaces it rather than adding to it.
    #[test]
    fn minimal_scenario_expands() {
        let tests = expand(MINIMAL, None).unwrap();
        assert!(tests.contains("fnt()"), "unexpected expansion: {tests}");
        assert!(
            tests.contains(
                ".delay(10u64).press(0u8,0u8).delay(10u64).release(0u8,0u8).expect_keys_with_mods(0u8,[])"
            ),
            "unexpected expansion: {tests}"
        );

        let doc = MINIMAL.replace("{ press = [0, 0] }", "{ delay = 99 }, { press = [0, 0] }");
        let tests = expand(&doc, None).unwrap();
        assert!(
            tests.contains(".delay(99u64).press(0u8,0u8)"),
            "unexpected expansion: {tests}"
        );
    }

    #[test]
    fn rmk_section_is_rejected() {
        let doc = format!("{MINIMAL}\n[rmk]\ncombo_max_num = 16\n");
        let err = expand(&doc, None).expect_err("expected error");
        assert!(err.contains("[rmk]"), "unexpected error: {err}");
    }

    #[test]
    fn unknown_step_op_is_rejected() {
        let doc = MINIMAL.replace("{ press = [0, 0] }", "{ pres = [0, 0] }");
        let err = expand(&doc, None).expect_err("expected error");
        assert!(
            err.contains("steps[0]") && err.contains("pres"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn out_of_range_position_is_rejected() {
        let doc = MINIMAL.replace("{ press = [0, 0] }", "{ press = [0, 3] }");
        let err = expand(&doc, None).expect_err("expected error");
        assert!(err.contains("1x1 matrix"), "unexpected error: {err}");
    }

    /// A real `keyboard.toml` works as a board: its hardware sections resolve
    /// harmlessly and `[input_device]` still declares the encoders.
    #[test]
    fn base_keyboard_toml_resolves_with_hardware_sections() {
        let base = "[keyboard]\nname = \"real\"\nvendor_id = 0x4b4d\nproduct_id = 0x4b31\nchip = \"nrf52840\"\n\
             [matrix]\nrow_pins = [\"r0\"]\ncol_pins = [\"c0\"]\n\
             [ble]\nenabled = true\n\
             [layout]\nrows = 1\ncols = 1\nmap = \"(0,0)\"\n\
             [[keymap.layer]]\nkeys = \"A\"\nencoders = [[\"Up\", \"Down\"]]\n\
             [[input_device.encoder]]\npin_a = \"a0\"\npin_b = \"b0\"\n";
        let doc = "[[test]]\nname = \"t\"\nsteps = [{ rotary_cw = 0 }]\n";
        let tests = expand(doc, Some(base)).unwrap();
        assert!(
            tests.contains(".encoders(") && tests.contains(".rotary_cw(0u8)"),
            "unexpected expansion: {tests}"
        );
    }

    /// Encoders spread across split halves still total up board-wide.
    #[test]
    fn split_board_encoders_are_summed() {
        let base = "[layout]\nrows = 1\ncols = 2\nmap = \"(0,0) (0,1)\"\n\
             [[keymap.layer]]\nkeys = \"A B\"\nencoders = [[\"Up\", \"Down\"], [\"Left\", \"Right\"]]\n\
             [split]\nconnection = \"ble\"\n\
             [split.central]\nrows = 1\ncols = 1\nrow_offset = 0\ncol_offset = 0\n\
             matrix = { row_pins = [\"r0\"], col_pins = [\"c0\"] }\n\
             [[split.central.input_device.encoder]]\npin_a = \"a0\"\npin_b = \"b0\"\n\
             [[split.peripheral]]\nrows = 1\ncols = 1\nrow_offset = 0\ncol_offset = 1\n\
             matrix = { row_pins = [\"r1\"], col_pins = [\"c1\"] }\n\
             [[split.peripheral.input_device.encoder]]\npin_a = \"a1\"\npin_b = \"b1\"\n";
        // A two-encoder layer map only resolves if both halves are counted.
        let doc = "[[test]]\nname = \"t\"\nsteps = [{ rotary_ccw = 1 }]\n";
        let tests = expand(doc, Some(base)).unwrap();
        assert!(
            tests.contains(".rotary_ccw(1u8)"),
            "unexpected expansion: {tests}"
        );
    }

    /// A scenario may resize a real board: `[matrix]` pin counts are hardware,
    /// and simulation never checks them against `[layout]`.
    #[test]
    fn scenario_may_resize_a_real_board() {
        let base = "[matrix]\nrow_pins = [\"r0\", \"r1\"]\ncol_pins = [\"c0\", \"c1\", \"c2\"]\n\
             [layout]\nrows = 2\ncols = 3\nmap = \"(0,0) (0,1) (0,2)\\n(1,0) (1,1) (1,2)\"\n\
             [[keymap.layer]]\nkeys = \"A B C D E F\"\n";
        let doc = "[layout]\nrows = 1\ncols = 1\nmap = \"(0,0)\"\n\
             [[keymap.layer]]\nkeys = \"A\"\n\
             [[test]]\nname = \"t\"\nsteps = [{ press = [0, 0] }]\n";
        let tests = expand(doc, Some(base)).unwrap();
        assert!(
            tests.contains("builder::<1usize,1usize,1usize>"),
            "unexpected expansion: {tests}"
        );
    }
}
