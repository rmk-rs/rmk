//! End-to-end hardware test for `rynk`.
//!
//! Runs against USB by default; pass `ble` for BLE. Setters write a different
//! value, verify it, then restore the original. Exits non-zero if any command
//! hits a transport error or a verify mismatch (device rejections and
//! capability-gated commands are expected on some hardware, so they're logged
//! but don't fail the run).
//!
//! ```text
//! cargo run -p rynk --example hw_test            # USB vendor bulk (default)
//! cargo run -p rynk --example hw_test -- ble     # BLE GATT
//! ```
//!
//! TODO: once `rynk-cli` exists, promote this sweep to a `rynk doctor`
//! subcommand — it's end-user value ("is my keyboard's Rynk working?"), and
//! moving it empties the root package's examples/ and its transport dev-deps.

use std::fmt::Debug;
use std::time::Duration;

use embassy_futures::select::{Either, select};
use log::{error, info, warn};
use rynk::rmk_types::action::EncoderAction;
use rynk::rmk_types::combo::Combo;
use rynk::rmk_types::morse::MorseProfile;
use rynk::rmk_types::protocol::rynk::{MacroData, StorageResetMode};
use rynk::{Client, RynkDevice, RynkHostError};
use rynk_ble::BleDevice;
use rynk_usb::UsbDevice;

/// Bounds the handshake so a silent peer can't hang the sweep.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

/// Shared failure arm: a device rejection or a capability gate is expected on
/// some hardware; anything else fails the run.
fn note_error(fails: &mut u32, label: &str, e: RynkHostError) {
    match e {
        RynkHostError::Rejected(e) => warn!("  ⚠ {label} rejected: {e:?}"),
        RynkHostError::Unsupported(_, why) => info!("  ({label} skipped: {why})"),
        e => {
            error!("  ✗ {label} {e}");
            *fails += 1;
        }
    }
}

/// Log a value-returning command.
fn report<V: Debug>(fails: &mut u32, label: &str, res: Result<V, RynkHostError>) {
    match res {
        Ok(v) => info!("  ✓ {label:<22} {v:?}"),
        Err(e) => note_error(fails, label, e),
    }
}

/// Log a unit-returning command.
fn ack(fails: &mut u32, label: &str, res: Result<(), RynkHostError>) {
    match res {
        Ok(()) => info!("  ✓ {label}"),
        Err(e) => note_error(fails, label, e),
    }
}

/// Compare a setter's readback against the value written; count mismatches and
/// errors as failures.
fn verify<V: Debug + PartialEq>(fails: &mut u32, label: &str, expected: &V, reget: Result<V, RynkHostError>) {
    match reget {
        Ok(actual) if actual == *expected => info!("  ↺ {label:<18} {expected:?} == {actual:?}"),
        Ok(actual) => {
            error!("  ✗ {label:<18} {expected:?} != {actual:?}");
            *fails += 1;
        }
        Err(e) => {
            error!("  ✗ {label:<18} {e}");
            *fails += 1;
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .format_target(false)
        .init();

    // Discovery is the one transport-specific call; connect + the command sweep
    // below are generic over `RynkDevice`.
    match std::env::args().nth(1).as_deref().unwrap_or("usb") {
        "usb" => run_first("USB vendor bulk", UsbDevice::discover().await?, false).await,
        "ble" => run_first("BLE GATT", BleDevice::discover().await?, true).await,
        other => Err(format!("unknown transport {other:?}; use 'usb' (default) or 'ble'").into()),
    }
}

/// pick the first discovered device → connect → run the command sweep, generic
/// over any [`RynkDevice`]. A real picker would let the user choose from the full
/// list; discovery itself is each transport's own inherent call.
async fn run_first<D: RynkDevice>(
    what: &str,
    devices: Vec<D>,
    over_ble: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("connecting over {what}…");
    let count = devices.len();
    let device = devices
        .into_iter()
        .next()
        .ok_or_else(|| format!("no Rynk keyboard found over {what}"))?;
    if count > 1 {
        info!("{count} keyboards found; connecting to {}", device.label());
    }
    // The lifecycle's `connect` is runtime-free and carries no handshake timeout;
    // bound it here so a silent peer can't hang the tool.
    let (client, mut driver) = tokio::time::timeout(HANDSHAKE_TIMEOUT, device.connect())
        .await
        .map_err(|_| format!("handshake timed out over {what}"))??;
    match select(driver.run(&client), run_all(&client, over_ble)).await {
        Either::First(err) => Err(format!("link died during the sweep: {err}").into()),
        Either::Second(res) => res,
    }
}

/// Exercise non-reboot Rynk commands.
async fn run_all(client: &Client, over_ble: bool) -> Result<(), Box<dyn std::error::Error>> {
    let caps = client.get_capabilities().await?;
    let version = client.get_version().await?;
    let mut fails = 0u32;
    info!(
        "connected over {} (protocol v{}.{}) — keymap {}×{}×{}, {} combos, {} forks, {} morse, ble={}",
        if over_ble { "BLE" } else { "USB" },
        version.major,
        version.minor,
        caps.num_layers,
        caps.num_rows,
        caps.num_cols,
        caps.max_combos,
        caps.max_forks,
        caps.max_morse,
        caps.ble_enabled,
    );

    info!("── system ──");
    report(&mut fails, "get_device_info", client.get_device_info().await);

    info!("── keymap ──");
    // Read, set a different value, verify, restore.
    match client.get_default_layer().await {
        // Only round-trip values we can restore.
        Ok(orig) if (orig as usize) < caps.num_layers as usize => {
            info!("  ✓ {:<22} {orig}", "get default_layer");
            let new = orig.wrapping_add(1) % caps.num_layers.max(1);
            if new != orig {
                ack(
                    &mut fails,
                    &format!("set default_layer ={new}"),
                    client.set_default_layer(new).await,
                );
                verify(&mut fails, "default_layer", &new, client.get_default_layer().await);
                ack(
                    &mut fails,
                    &format!("restore default_layer ={orig}"),
                    client.set_default_layer(orig).await,
                );
                verify(&mut fails, "default_layer", &orig, client.get_default_layer().await);
            } else {
                ack(&mut fails, "set default_layer", client.set_default_layer(orig).await);
            }
        }
        Ok(orig) => warn!(
            "  ⚠ {:<20} out-of-range default_layer {orig} (num_layers={}); skipping round-trip",
            "get default_layer", caps.num_layers
        ),
        other => report(&mut fails, "get default_layer", other),
    }
    // Read the whole keymap by walking the single-key endpoint.
    let mut total = 0usize;
    let mut scan_error = None;
    'scan: for layer in 0..caps.num_layers {
        for row in 0..caps.num_rows {
            for col in 0..caps.num_cols {
                match client.get_key(layer, row, col).await {
                    Ok(_) => total += 1,
                    Err(e) => {
                        scan_error = Some(e);
                        break 'scan;
                    }
                }
            }
        }
    }
    if let Some(e) = scan_error {
        error!("  ✗ scan keymap {e}");
        fails += 1;
    } else {
        info!(
            "  ✓ {:<22} {total} keys across {} layers",
            "scan keymap", caps.num_layers
        );
    }
    // Set L0(0,0), verify, restore.
    let key_00 = client.get_key(0, 0, 0).await;
    let key_01 = client.get_key(0, 0, 1).await;
    match (key_00, key_01) {
        (Ok(orig), Ok(alt)) if alt != orig => {
            info!("  ✓ {:<22} {orig:?}", "get key L0(0,0)");
            ack(
                &mut fails,
                &format!("set key L0(0,0) ={alt:?}"),
                client.set_key(0, 0, 0, alt).await,
            );
            verify(&mut fails, "key L0(0,0)", &alt, client.get_key(0, 0, 0).await);
            ack(&mut fails, "restore key L0(0,0)", client.set_key(0, 0, 0, orig).await);
            verify(&mut fails, "key L0(0,0)", &orig, client.get_key(0, 0, 0).await);
        }
        (Ok(orig), _) => {
            info!("  ✓ {:<22} {orig:?}", "get key L0(0,0)");
            ack(&mut fails, "set key L0(0,0)", client.set_key(0, 0, 0, orig).await);
        }
        (other, _) => report(&mut fails, "get key L0(0,0)", other),
    }
    // Encoders: swap rotation actions, verify, restore.
    if caps.num_encoders == 0 {
        info!("  (no encoders configured — exercising dispatch at index 0)");
    }
    match client.get_encoder(0, 0).await {
        Ok(orig) => {
            info!("  ✓ {:<22} {orig:?}", "get_encoder 0/L0");
            let changed = EncoderAction::new(orig.counter_clockwise, orig.clockwise);
            if changed != orig {
                ack(
                    &mut fails,
                    "set_encoder 0/L0 (swap)",
                    client.set_encoder(0, 0, changed).await,
                );
                verify(&mut fails, "encoder 0/L0", &changed, client.get_encoder(0, 0).await);
                ack(&mut fails, "restore encoder 0/L0", client.set_encoder(0, 0, orig).await);
                verify(&mut fails, "encoder 0/L0", &orig, client.get_encoder(0, 0).await);
            } else {
                ack(
                    &mut fails,
                    "set_encoder 0/L0 (write-back)",
                    client.set_encoder(0, 0, orig).await,
                );
            }
        }
        other => {
            report(&mut fails, "get_encoder 0/L0", other);
            ack(
                &mut fails,
                "set_encoder 0/L0 (dispatch only)",
                client.set_encoder(0, 0, EncoderAction::default()).await,
            );
        }
    }

    info!("── macros ──");
    if caps.macro_space_size == 0 {
        info!("  (no macro storage — exercising dispatch at offset 0)");
    }
    report(&mut fails, "get_macro 0", client.get_macro(0).await);
    ack(
        &mut fails,
        "set_macro 0",
        client
            .set_macro(
                0,
                MacroData {
                    data: Default::default(),
                },
            )
            .await,
    );

    info!("── combos ──");
    // Combo: set a different valid combo, verify, restore.
    if caps.max_combos > 0 {
        match client.get_combo(0).await {
            Ok(orig) => {
                info!("  ✓ {:<22} {orig:?}", "get combo 0");
                let k0 = client.get_key(0, 0, 0).await.ok();
                let k1 = client.get_key(0, 1, 0).await.ok();
                if let (Some(k0), Some(k1)) = (k0, k1) {
                    let mut actions = heapless::Vec::new();
                    let _ = actions.push(k0);
                    let _ = actions.push(k1);
                    let changed = Combo {
                        actions,
                        output: k0,
                        layer: Some(1),
                    };
                    ack(
                        &mut fails,
                        "set combo 0 (mutate)",
                        client.set_combo(0, changed.clone()).await,
                    );
                    verify(&mut fails, "combo 0", &changed, client.get_combo(0).await);
                    ack(&mut fails, "restore combo 0", client.set_combo(0, orig.clone()).await);
                    verify(&mut fails, "combo 0", &orig, client.get_combo(0).await);
                } else {
                    ack(
                        &mut fails,
                        "set combo 0 (write-back)",
                        client.set_combo(0, orig.clone()).await,
                    );
                }
            }
            other => report(&mut fails, "get combo 0", other),
        }
    }

    info!("── forks ──");
    // Fork: mutate one output, verify, restore.
    if caps.max_forks > 0 {
        match client.get_fork(0).await {
            Ok(orig) => {
                info!("  ✓ {:<22} {orig:?}", "get fork 0");
                let key = client.get_key(0, 0, 1).await.ok();
                match key {
                    Some(key) if key != orig.positive_output => {
                        let mut changed = orig;
                        changed.positive_output = key;
                        ack(
                            &mut fails,
                            &format!("set fork 0 (positive_output={key:?})"),
                            client.set_fork(0, changed).await,
                        );
                        verify(&mut fails, "fork 0", &changed, client.get_fork(0).await);
                        ack(&mut fails, "restore fork 0", client.set_fork(0, orig).await);
                        verify(&mut fails, "fork 0", &orig, client.get_fork(0).await);
                    }
                    _ => ack(&mut fails, "set fork 0 (write-back)", client.set_fork(0, orig).await),
                }
            }
            other => report(&mut fails, "get fork 0", other),
        }
    }

    info!("── morse ──");
    // Morse: mutate hold timeout, verify, restore.
    if caps.max_morse > 0 {
        match client.get_morse(0).await {
            Ok(orig) => {
                info!("  ✓ {:<22} {orig:?}", "get morse 0");
                let mut changed = orig.clone();
                changed.profile = MorseProfile::const_default().with_hold_timeout_ms(Some(180));
                if changed != orig {
                    ack(
                        &mut fails,
                        "set morse 0 (mutate)",
                        client.set_morse(0, changed.clone()).await,
                    );
                    verify(&mut fails, "morse 0", &changed, client.get_morse(0).await);
                    ack(&mut fails, "restore morse 0", client.set_morse(0, orig.clone()).await);
                    verify(&mut fails, "morse 0", &orig, client.get_morse(0).await);
                } else {
                    ack(&mut fails, "set morse 0 (write-back)", client.set_morse(0, orig).await);
                }
            }
            other => report(&mut fails, "get morse 0", other),
        }
    }

    info!("── behavior ──");
    match client.get_behavior().await {
        Ok(orig) => {
            info!("  ✓ {:<22} {orig:?}", "get behavior");
            let mut changed = orig;
            changed.combo_timeout_ms = orig.combo_timeout_ms.wrapping_add(5);
            ack(
                &mut fails,
                &format!("set behavior (combo_timeout={})", changed.combo_timeout_ms),
                client.set_behavior(changed).await,
            );
            verify(&mut fails, "behavior", &changed, client.get_behavior().await);
            ack(&mut fails, "restore behavior", client.set_behavior(orig).await);
            verify(&mut fails, "behavior", &orig, client.get_behavior().await);
        }
        other => report(&mut fails, "get behavior", other),
    }

    info!("── status ──");
    report(&mut fails, "get_current_layer", client.get_current_layer().await);
    report(&mut fails, "get_matrix_state", client.get_matrix_state().await);
    report(&mut fails, "get_wpm", client.get_wpm().await);
    report(&mut fails, "get_sleep_state", client.get_sleep_state().await);
    report(&mut fails, "get_led_indicator", client.get_led_indicator().await);
    // Called unconditionally: the client's own capability gate answers
    // `Unsupported`, which logs as a skip.
    report(&mut fails, "get_battery_status", client.get_battery_status().await);
    // The slot count drives iteration; `.max(1)` probes the gate once on
    // non-split builds.
    for slot in 0..caps.num_split_peripherals.max(1) {
        report(
            &mut fails,
            &format!("get_peripheral_status {slot}"),
            client.get_peripheral_status(slot).await,
        );
    }

    info!("── connection ──");
    report(&mut fails, "get_connection_type", client.get_connection_type().await);
    report(
        &mut fails,
        "get_connection_status",
        client.get_connection_status().await,
    );
    match client.get_ble_status().await {
        Ok(s) => {
            info!("  ✓ {:<22} {s:?}", "get_ble_status");
            // Re-selecting the active profile is a no-op.
            ack(
                &mut fails,
                &format!("switch_ble_profile {} (already active)", s.profile),
                client.switch_ble_profile(s.profile).await,
            );
            // Clearing bonds is not restorable.
            info!("  (skipping clear_ble_profile: deleting a bond is unrestorable)");
        }
        other => {
            report(&mut fails, "get_ble_status", other);
            info!("  (active profile unknown — skipping switch_ble_profile / clear_ble_profile)");
        }
    }

    info!("── storage ──");
    if over_ble {
        // A storage wipe includes bonds.
        info!("  (over BLE — skipping storage_reset: a wipe would drop this BLE link)");
    } else {
        // Exercise the command path without wiping user data.
        ack(
            &mut fails,
            "storage_reset(LayoutOnly) [expect Unimplemented until mode-aware reset lands]",
            client.storage_reset(StorageResetMode::LayoutOnly).await,
        );
    }

    info!("── topics (best-effort drain) ──");
    // Drain queued topics and brief late arrivals.
    let mut topic_count = 0;
    while let Ok(event) = tokio::time::timeout(std::time::Duration::from_millis(300), client.next_topic()).await {
        info!("  ⤷ topic {event:?}");
        topic_count += 1;
    }
    if topic_count == 0 {
        info!("  (none received — topics are event-driven, e.g. key presses)");
    }

    if fails == 0 {
        info!(
            "done — command sweep passed over {}.",
            if over_ble { "BLE" } else { "USB" }
        );
        Ok(())
    } else {
        Err(format!("{fails} command(s) failed").into())
    }
}
