//! Rynk test support: shared serde-test helpers for the submodules, plus
//! the cross-module wire-format tests.
//!
//! Schema drift detection across two golden files: `wire_values.snap` holds
//! one postcard-encoded exemplar per wire type; `wire_frames.snap` holds one
//! full frame (header + payload) per protocol message. Any field reorder /
//! type change / variant renumber / CMD renumber flips the bytes and fails
//! CI. If the change is intentional, bump `ProtocolVersion::CURRENT` and
//! regenerate the snapshots.

extern crate alloc;

use alloc::vec;

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use super::*;
use crate::action::{Action, EncoderAction, KeyAction, KeyboardAction, LightAction};
use crate::battery::{BatteryStatus, ChargeState};
use crate::ble::{BleState, BleStatus};
use crate::combo::Combo;
use crate::connection::{ConnectionStatus, ConnectionType, UsbState};
use crate::fork::{Fork, StateBits};
use crate::keycode::{ConsumerKey, HidKeyCode, KeyCode, SpecialKey, SystemControlKey};
use crate::led_indicator::LedIndicator;
use crate::modifier::ModifierCombination;
use crate::morse::{Morse, MorseMode, MorseProfile, TAP};
use crate::mouse_button::MouseButtons;

/// Buffer size used by round-trip / max-size helpers.
///
/// Sized at twice the type's declared `POSTCARD_MAX_SIZE` plus a small
/// fixed slack so that:
/// - under feature configurations with a large `MAX_BULK_ITEMS`, max-capacity
///   bulk payloads still fit comfortably;
/// - an under-counted manual `MaxSize` impl produces a clear assertion
///   failure in `assert_max_size_bound` instead of a `SerializeBufferFull`
///   panic.
fn buffer_capacity<T: MaxSize>() -> usize {
    T::POSTCARD_MAX_SIZE.saturating_mul(2).saturating_add(64)
}

/// Postcard round-trip helper used by every submodule's tests.
pub fn round_trip<T>(val: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq + core::fmt::Debug + MaxSize,
{
    let mut buf = vec![0u8; buffer_capacity::<T>()];
    let bytes = postcard::to_slice(val, &mut buf).expect("serialize");
    let decoded: T = postcard::from_bytes(bytes).expect("deserialize");
    assert_eq!(&decoded, val);
    decoded
}

/// Assert that `val` serializes within its declared `POSTCARD_MAX_SIZE`.
/// Use alongside `round_trip` in max-capacity tests to catch
/// under-counted manual `MaxSize` impls.
pub fn assert_max_size_bound<T>(val: &T)
where
    T: Serialize + MaxSize,
{
    let mut buf = vec![0u8; buffer_capacity::<T>()];
    let bytes = postcard::to_slice(val, &mut buf).expect("serialize");
    assert!(
        bytes.len() <= T::POSTCARD_MAX_SIZE,
        "{} encoded to {} bytes but POSTCARD_MAX_SIZE = {}",
        core::any::type_name::<T>(),
        bytes.len(),
        T::POSTCARD_MAX_SIZE,
    );
}

mod snapshot {
    extern crate alloc;
    extern crate std;

    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;
    use std::path::PathBuf;
    use std::{env, fs};

    /// Format a byte slice as lowercase, space-separated hex.
    pub fn hex(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 3);
        for (i, b) in bytes.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    /// Build the snapshot text for a list of (label, encoded bytes) pairs.
    /// `title` heads the file and `blurb` (already `#`-prefixed lines) describes
    /// its entries; `test_filter` names the test in the regenerate hint.
    pub fn format_value_snapshot(
        rel_path: &str,
        title: &str,
        blurb: &str,
        test_filter: &str,
        entries: &[(&str, &[u8])],
    ) -> String {
        let mut sorted: Vec<&(&str, &[u8])> = entries.iter().collect();
        sorted.sort_by_key(|(label, _)| *label);

        let label_width = sorted.iter().map(|(l, _)| l.len()).max().unwrap_or(0);

        let mut out = String::new();
        out.push_str(&format!(
            "# {title} — DO NOT edit by hand.\n\
             # File: {rel_path}\n\
             {blurb}\n\
             #   UPDATE_SNAPSHOTS=1 cargo test -p rmk-types --features host {test_filter}\n\
             # Format: <label>  <hex bytes>\n\
             \n",
        ));
        for (label, bytes) in sorted {
            out.push_str(&format!("{:width$}  {}\n", label, hex(bytes), width = label_width));
        }
        out
    }

    /// Compare actual snapshot text against the on-disk file.
    /// When `UPDATE_SNAPSHOTS` is set, write the file instead.
    pub fn assert_snapshot(rel_path: &str, actual: String) {
        assert_snapshot_at(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("src/protocol/rynk")
                .join(rel_path),
            actual,
        );
    }

    /// [`assert_snapshot`] for a generated file at an arbitrary path (e.g. the
    /// protocol reference under `docs/`).
    pub fn assert_snapshot_at(path: PathBuf, actual: String) {
        if env::var_os("UPDATE_SNAPSHOTS").is_some() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .unwrap_or_else(|e| panic!("create snapshot dir {}: {}", parent.display(), e));
            }
            fs::write(&path, &actual).unwrap_or_else(|e| panic!("write snapshot {}: {}", path.display(), e));
            return;
        }

        let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "missing snapshot {} ({}). Run with UPDATE_SNAPSHOTS=1 to create.",
                path.display(),
                e,
            )
        });

        if expected != actual {
            panic!(
                "snapshot mismatch: {}\n\
                 --- expected ---\n{}\
                 --- actual ---\n{}\
                 If intentional, regenerate with UPDATE_SNAPSHOTS=1 and bump ProtocolVersion::CURRENT.",
                path.display(),
                expected,
                actual,
            );
        }
    }
}

#[test]
fn round_trip_rynk_error_and_result() {
    round_trip(&RynkError::Malformed);
    round_trip(&RynkError::NotReady);
    round_trip(&RynkError::StorageFault);
    round_trip(&RynkError::Internal);
    round_trip(&RynkError::Unimplemented);
    round_trip(&RynkError::Invalid);
    round_trip(&RynkError::UnknownCmd);
    round_trip(&RynkError::Locked);
    round_trip(&RynkError::Busy);
    let ok: Result<(), RynkError> = Ok(());
    let err: Result<(), RynkError> = Err(RynkError::StorageFault);
    let _ = round_trip(&ok);
    let _ = round_trip(&err);
}

fn encode<T: serde::Serialize>(val: &T) -> alloc::vec::Vec<u8> {
    let mut buf = [0u8; 256];
    let bytes = postcard::to_slice(val, &mut buf).expect("encode");
    bytes.to_vec()
}

/// Frames are frozen only on `host`; see [`wire_frames_locked`].
#[cfg(feature = "host")]
fn encode_frame<T: serde::Serialize>(cmd: Cmd, seq: u8, val: &T) -> alloc::vec::Vec<u8> {
    let mut buf = [0u8; 256];
    let n = super::message::encode_frame(&mut buf, RynkHeader { cmd, seq }, val).expect("frame");
    buf[..n].to_vec()
}

/// Composite wire exemplars shared by both the type and frame snapshots, so
/// a combo / fork / morse / capabilities value encodes to the same bytes in
/// both files. Distinct, ascending per-field values let a field reorder flip
/// the bytes.
struct Exemplars {
    matrix: MatrixState,
    capabilities: DeviceCapabilities,
    device_info: DeviceInfo,
    behavior: BehaviorConfig,
    connection: ConnectionStatus,
    state_bits: StateBits,
    combo: Combo,
    fork: Fork,
    morse: Morse,
    macro_data: MacroData,
    encoder: EncoderAction,
    battery: BatteryStatus,
    layout: LayoutChunk,
}

fn exemplars() -> Exemplars {
    let mut bitmap: heapless::Vec<u8, MATRIX_BITMAP_SIZE> = heapless::Vec::new();
    bitmap.extend_from_slice(&[0x05, 0x00, 0x20]).unwrap();
    let matrix = MatrixState { pressed_bitmap: bitmap };

    // Distinct ascending per-field values so a field reorder flips bytes.
    let capabilities = DeviceCapabilities {
        num_layers: 1,
        num_rows: 2,
        num_cols: 3,
        num_encoders: 4,
        max_combos: 5,
        max_combo_keys: 6,
        macro_space_size: 7,
        max_morse: 8,
        max_patterns_per_key: 9,
        max_forks: 10,
        storage_enabled: true,
        lighting_enabled: false,
        is_split: true,
        num_split_peripherals: 11,
        ble_enabled: false,
        num_ble_profiles: 12,
        max_payload_size: 13,
        max_bulk_keys: 14,
        max_bulk_items: 15,
        macro_chunk_size: 16,
        bulk_transfer_supported: true,
    };
    // Ascending version/id values; distinct strings so a field swap shows.
    let device_info = DeviceInfo {
        rmk_version: FirmwareVersion {
            major: 1,
            minor: 2,
            patch: 3,
        },
        vendor_id: 4,
        product_id: 5,
        manufacturer: heapless::String::try_from("RMK").unwrap(),
        product_name: heapless::String::try_from("RMK Keyboard").unwrap(),
        serial_number: heapless::String::try_from("rynk:0001").unwrap(),
    };
    // Quick-tap sits in the u64's high bits, so the profile also exercises a
    // long-varint encoding.
    let behavior = BehaviorConfig {
        combo_timeout_ms: 50,
        oneshot_timeout_ms: 60,
        tap_interval_ms: 70,
        tap_capslock_interval_ms: 80,
        morse_default_profile: MorseProfile::new(Some(true), Some(MorseMode::HoldOnOtherPress), Some(90), Some(100))
            .with_quick_tap_timeout_ms(Some(110)),
        morse_prior_idle_time_ms: 120,
    };
    let connection = ConnectionStatus {
        usb: UsbState::Configured,
        ble: BleStatus {
            profile: 1,
            state: BleState::Advertising,
        },
        preferred: ConnectionType::Ble,
    };
    // All three sub-bitfields distinct so a StateBits field swap shows.
    let state_bits = StateBits::new_from(
        ModifierCombination::LCTRL,
        LedIndicator::CAPS_LOCK,
        MouseButtons::BUTTON1,
    );
    let combo = Combo::new(
        [KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A)))],
        KeyAction::Morse(1),
        Some(2),
    );
    let fork = Fork::new(
        KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A))),
        KeyAction::No,
        KeyAction::Morse(2),
        state_bits,
        StateBits::default(),
        ModifierCombination::LSHIFT,
        true,
    );
    // Pins Morse's custom serde shape: (MorseProfile, Vec<(u16, Action)>).
    let mut morse_actions = heapless::LinearMap::new();
    morse_actions
        .insert(TAP, Action::Key(KeyCode::Hid(HidKeyCode::A)))
        .unwrap();
    let morse = Morse {
        profile: MorseProfile::const_default(),
        actions: morse_actions,
    };
    let mut macro_bytes = heapless::Vec::new();
    macro_bytes.extend_from_slice(&[0x01, 0x02, 0x03]).unwrap();
    let macro_data = MacroData { data: macro_bytes };
    let encoder = EncoderAction::new(KeyAction::Morse(3), KeyAction::No);
    // A page shorter than the chunk size, with a `total_len` that outgrows it and
    // takes two varint bytes, so swapping the two fields flips the bytes.
    let mut layout_bytes: heapless::Vec<u8, RYNK_BLE_CHUNK_SIZE> = heapless::Vec::new();
    layout_bytes.extend_from_slice(&[0x0a, 0x0b, 0x0c]).unwrap();
    let layout = LayoutChunk {
        total_len: 300,
        bytes: layout_bytes,
    };

    Exemplars {
        matrix,
        capabilities,
        device_info,
        behavior,
        connection,
        state_bits,
        combo,
        fork,
        morse,
        macro_data,
        encoder,
        battery: BatteryStatus::Available {
            charge_state: ChargeState::Discharging,
            level: Some(85),
        },
        layout,
    }
}

/// Lock down postcard's actual byte encoding for stability-critical
/// values. A diff in this snapshot indicates wire-format drift; if
/// intentional, regenerate the snapshot and bump `ProtocolVersion::CURRENT`.
///
/// One exemplar per Rynk wire type, plus every variant of the positional
/// enums (`KeyAction`, `Action`, and the status enums) so a reordered or
/// inserted variant flips the bytes. Postcard tags enums by declaration
/// order, *not* the `#[repr]` discriminant, so the keycode exemplars also
/// pin variant ordinals. Structs use distinct per-field values so a field
/// swap is caught too. Only feature-independent values belong here: the
/// gated `Action::Steno`, the `bulk` request/response payloads, and
/// `PeripheralStatus` are excluded so every `rynk` feature set yields the
/// same snapshot. Full frames are pinned separately in `wire_frames_locked`.
#[test]
fn wire_values_locked() {
    let ex = exemplars();

    // Values-only exemplars (no frame counterpart).
    let mut unlock_keys = heapless::Vec::new();
    unlock_keys.push((1, 2)).unwrap();
    unlock_keys.push((3, 4)).unwrap();
    let lock_status = LockStatus {
        locked: true,
        unlocking: false,
        remaining_keys: 2,
        key_positions: unlock_keys,
    };
    let profile = MorseProfile::new(None, Some(MorseMode::Normal), Some(200), Some(150));

    let entries: alloc::vec::Vec<(&str, alloc::vec::Vec<u8>)> = alloc::vec![
        // --- Response envelope + connection ---
        ("ConnectionType::Ble", encode(&ConnectionType::Ble)),
        ("ConnectionType::Usb", encode(&ConnectionType::Usb)),
        (
            "Result<(),RynkError>::Err(StorageFault)",
            encode::<Result<(), RynkError>>(&Err(RynkError::StorageFault)),
        ),
        ("Result<(),RynkError>::Ok", encode::<Result<(), RynkError>>(&Ok(()))),
        ("RynkError::Internal", encode(&RynkError::Internal)),
        ("RynkError::Invalid", encode(&RynkError::Invalid)),
        ("RynkError::Locked", encode(&RynkError::Locked)),
        ("RynkError::Malformed", encode(&RynkError::Malformed)),
        ("RynkError::NotReady", encode(&RynkError::NotReady)),
        ("RynkError::StorageFault", encode(&RynkError::StorageFault)),
        ("RynkError::Unimplemented", encode(&RynkError::Unimplemented)),
        ("RynkError::UnknownCmd", encode(&RynkError::UnknownCmd)),
        ("RynkError::Busy", encode(&RynkError::Busy)),
        // --- KeyAction: every variant tag (positional) ---
        ("KeyAction::No", encode(&KeyAction::No)),
        ("KeyAction::Transparent", encode(&KeyAction::Transparent)),
        (
            "KeyAction::Single(Action::Key(Hid(A)))",
            encode(&KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A)))),
        ),
        ("KeyAction::Tap(Action::No)", encode(&KeyAction::Tap(Action::No))),
        (
            "KeyAction::TapHold(Key(A),LayerOn(3))",
            encode(&KeyAction::TapHold(
                Action::Key(KeyCode::Hid(HidKeyCode::A)),
                Action::LayerOn(3),
                u8::MAX,
            )),
        ),
        ("KeyAction::Morse(3)", encode(&KeyAction::Morse(3))),
        // --- Action: every feature-independent variant tag (positional) ---
        ("Action::No", encode(&Action::No)),
        ("Action::Key(Hid(A))", encode(&Action::Key(KeyCode::Hid(HidKeyCode::A)))),
        (
            "Action::Modifier(LCtrl)",
            encode(&Action::Modifier(ModifierCombination::LCTRL))
        ),
        (
            "Action::KeyWithModifier(A,LShift)",
            encode(&Action::KeyWithModifier(HidKeyCode::A, ModifierCombination::LSHIFT)),
        ),
        ("Action::LayerOn(1)", encode(&Action::LayerOn(1))),
        (
            "Action::LayerOnWithModifier(2,LCtrl)",
            encode(&Action::LayerOnWithModifier(2, ModifierCombination::LCTRL)),
        ),
        ("Action::LayerOff(3)", encode(&Action::LayerOff(3))),
        ("Action::LayerToggle(4)", encode(&Action::LayerToggle(4))),
        ("Action::DefaultLayer(5)", encode(&Action::DefaultLayer(5))),
        ("Action::LayerToggleOnly(6)", encode(&Action::LayerToggleOnly(6))),
        ("Action::TriLayerLower", encode(&Action::TriLayerLower)),
        ("Action::TriLayerUpper", encode(&Action::TriLayerUpper)),
        ("Action::TriggerMacro(7)", encode(&Action::TriggerMacro(7))),
        ("Action::OneShotLayer(8)", encode(&Action::OneShotLayer(8))),
        (
            "Action::OneShotModifier(LAlt)",
            encode(&Action::OneShotModifier(ModifierCombination::LALT))
        ),
        ("Action::OneShotKey(Hid(B))", encode(&Action::OneShotKey(HidKeyCode::B))),
        ("Action::Light(RgbTog)", encode(&Action::Light(LightAction::RgbTog))),
        (
            "Action::KeyboardControl(Bootloader)",
            encode(&Action::KeyboardControl(KeyboardAction::Bootloader)),
        ),
        (
            "Action::Special(GraveEscape)",
            encode(&Action::Special(SpecialKey::GraveEscape))
        ),
        ("Action::User(9)", encode(&Action::User(9))),
        // --- KeyCode discriminants (postcard tags by ordinal, not repr) ---
        ("KeyCode::Hid(A)", encode(&KeyCode::Hid(HidKeyCode::A))),
        (
            "KeyCode::Consumer(VolumeIncrement)",
            encode(&KeyCode::Consumer(ConsumerKey::VolumeIncrement)),
        ),
        (
            "KeyCode::SystemControl(Sleep)",
            encode(&KeyCode::SystemControl(SystemControlKey::Sleep))
        ),
        // --- Bitfields: pin LSB bit order ---
        (
            "ModifierCombination(LCtrl|RGui)",
            encode(&(ModifierCombination::LCTRL | ModifierCombination::RGUI)),
        ),
        (
            "LedIndicator(Num|Scroll)",
            encode(&(LedIndicator::NUM_LOCK | LedIndicator::SCROLL_LOCK))
        ),
        (
            "MouseButtons(B1|B8)",
            encode(&(MouseButtons::BUTTON1 | MouseButtons::BUTTON8))
        ),
        ("MorseProfile(Normal,200,150)", encode(&profile)),
        // --- Keymap / encoder / behavior config payloads ---
        (
            "KeyPosition{layer:0,row:5,col:13}",
            encode(&KeyPosition {
                layer: 0,
                row: 5,
                col: 13
            })
        ),
        ("EncoderAction{Morse(3),No}", encode(&ex.encoder)),
        ("Combo{[Single(A)],Morse(1),L2}", encode(&ex.combo)),
        ("Fork{Single(A),No,Morse(2)}", encode(&ex.fork)),
        ("StateBits{LCtrl,Caps,B1}", encode(&ex.state_bits)),
        ("Morse{TAP->Key(A)}", encode(&ex.morse)),
        ("MacroData{[0x01,0x02,0x03]}", encode(&ex.macro_data)),
        // --- Status / system responses ---
        ("MatrixState{[0x05,0x00,0x20]}", encode(&ex.matrix)),
        ("DeviceCapabilities{1..16}", encode(&ex.capabilities)),
        ("DeviceInfo{1.2.3,4,5,RMK,..}", encode(&ex.device_info)),
        ("BehaviorConfig{50..120}", encode(&ex.behavior)),
        ("ConnectionStatus{Configured,{1,Adv},Ble}", encode(&ex.connection)),
        ("ProtocolVersion{1,0}", encode(&ProtocolVersion { major: 1, minor: 0 })),
        ("ProtocolVersion::CURRENT", encode(&ProtocolVersion::CURRENT)),
        ("LockStatus{true,false,2,[(1,2),(3,4)]}", encode(&lock_status),),
        ("BatteryStatus::Unavailable", encode(&BatteryStatus::Unavailable)),
        ("BatteryStatus::Available{Discharging,85}", encode(&ex.battery)),
        ("ChargeState::Charging", encode(&ChargeState::Charging)),
        ("ChargeState::Discharging", encode(&ChargeState::Discharging)),
        ("ChargeState::Unknown", encode(&ChargeState::Unknown)),
        ("BleState::Advertising", encode(&BleState::Advertising)),
        ("BleState::Connected", encode(&BleState::Connected)),
        ("BleState::Inactive", encode(&BleState::Inactive)),
        (
            "BleStatus{2,Connected}",
            encode(&BleStatus {
                profile: 2,
                state: BleState::Connected
            })
        ),
        ("UsbState::Disabled", encode(&UsbState::Disabled)),
        ("UsbState::Enabled", encode(&UsbState::Enabled)),
        ("UsbState::Configured", encode(&UsbState::Configured)),
        ("UsbState::Suspended", encode(&UsbState::Suspended)),
        ("StorageResetMode::Full", encode(&StorageResetMode::Full)),
        ("StorageResetMode::LayoutOnly", encode(&StorageResetMode::LayoutOnly)),
        ("LayoutChunk{300,[0x0a,0x0b,0x0c]}", encode(&ex.layout)),
        // --- Request payloads: pin field order of the Get/Set structs ---
        (
            "SetKeyRequest{{0,5,13},Morse(7)}",
            encode(&SetKeyRequest {
                position: KeyPosition {
                    layer: 0,
                    row: 5,
                    col: 13
                },
                action: KeyAction::Morse(7),
            }),
        ),
        (
            "GetEncoderRequest{1,2}",
            encode(&GetEncoderRequest {
                encoder_id: 1,
                layer: 2
            })
        ),
        (
            "SetEncoderRequest{1,2,{Morse(3),No}}",
            encode(&SetEncoderRequest {
                encoder_id: 1,
                layer: 2,
                action: ex.encoder
            }),
        ),
        ("GetMacroRequest{256}", encode(&GetMacroRequest { offset: 256 })),
        (
            "SetMacroRequest{2,[0x01,0x02,0x03]}",
            encode(&SetMacroRequest {
                offset: 2,
                data: ex.macro_data.clone()
            }),
        ),
        (
            "SetComboRequest{3,combo}",
            encode(&SetComboRequest {
                index: 3,
                config: ex.combo.clone()
            })
        ),
        (
            "SetMorseRequest{0,morse}",
            encode(&SetMorseRequest {
                index: 0,
                config: ex.morse.clone()
            })
        ),
        (
            "SetForkRequest{2,fork}",
            encode(&SetForkRequest {
                index: 2,
                config: ex.fork
            })
        ),
    ];
    let view: alloc::vec::Vec<(&str, &[u8])> = entries.iter().map(|(l, b)| (*l, b.as_slice())).collect();

    let actual = snapshot::format_value_snapshot(
        "snapshots/wire_values.snap",
        "Wire-format TYPE snapshot",
        "# Each entry is the postcard byte encoding of one wire-type exemplar. A diff\n\
         # here means a type's payload encoding changed (field reorder, variant\n\
         # renumber, …). If intentional, bump ProtocolVersion::CURRENT and regenerate:",
        "wire_values",
        &view,
    );
    snapshot::assert_snapshot("snapshots/wire_values.snap", actual);
}

/// Lock down full Rynk frames — the 3-byte header plus postcard payload,
/// COBS-encoded with a trailing `0x00` delimiter — one per feature-independent
/// protocol message: every request, its `Ok` reply, a representative `Err`
/// reply, and every topic push. A diff means the wire format changed; if
/// intentional, regenerate and bump `ProtocolVersion::CURRENT`.
///
/// Requests and replies use SEQ 1 (a reply echoes its request's SEQ); topics
/// always use SEQ 0. The `GetVersion` probe and reply are frozen across all
/// majors. Payloads reuse the shared [`exemplars`], so a frame and its
/// bare-payload entry in `wire_values.snap` stay in lockstep.
///
/// Gated on `host`, the feature superset (`_ble` + `split` + `steno`): the file
/// then holds every gated row exactly once instead of dropping the rows a
/// lesser feature set can't name. Only `bulk` stays out —
/// its payload types differ between host and firmware. Regenerate with
/// `UPDATE_SNAPSHOTS=1 cargo test -p rmk-types --features host`.
#[cfg(feature = "host")]
#[test]
fn wire_frames_locked() {
    let ex = exemplars();

    // Request seq; a reply echoes it. Topics are always seq 0.
    const SEQ: u8 = 1;
    let key_pos = KeyPosition {
        layer: 0,
        row: 5,
        col: 13,
    };
    let set_key = SetKeyRequest {
        position: key_pos,
        action: KeyAction::Morse(7),
    };
    let led = LedIndicator::NUM_LOCK | LedIndicator::SCROLL_LOCK;
    let mut unlock_keys = heapless::Vec::new();
    unlock_keys.push((1, 2)).unwrap();
    unlock_keys.push((3, 4)).unwrap();
    let lock_status = LockStatus {
        locked: true,
        unlocking: false,
        remaining_keys: 2,
        key_positions: unlock_keys,
    };

    let entries: alloc::vec::Vec<(&str, alloc::vec::Vec<u8>)> = alloc::vec![
        // System (0x00xx).
        ("GetVersion request ()", encode_frame(Cmd::GetVersion, SEQ, &())),
        (
            "GetVersion reply Ok(CURRENT)",
            encode_frame(
                Cmd::GetVersion,
                SEQ,
                &Ok::<ProtocolVersion, RynkError>(ProtocolVersion::CURRENT)
            ),
        ),
        (
            "GetCapabilities request ()",
            encode_frame(Cmd::GetCapabilities, SEQ, &())
        ),
        (
            "GetCapabilities reply Ok(DeviceCapabilities{1..16})",
            encode_frame(
                Cmd::GetCapabilities,
                SEQ,
                &Ok::<DeviceCapabilities, RynkError>(ex.capabilities)
            ),
        ),
        ("Reboot request ()", encode_frame(Cmd::Reboot, SEQ, &())),
        (
            "Reboot reply Ok(())",
            encode_frame(Cmd::Reboot, SEQ, &Ok::<(), RynkError>(()))
        ),
        ("BootloaderJump request ()", encode_frame(Cmd::BootloaderJump, SEQ, &())),
        (
            "BootloaderJump reply Ok(())",
            encode_frame(Cmd::BootloaderJump, SEQ, &Ok::<(), RynkError>(())),
        ),
        (
            "StorageReset request StorageResetMode::Full",
            encode_frame(Cmd::StorageReset, SEQ, &StorageResetMode::Full)
        ),
        (
            "StorageReset reply Ok(())",
            encode_frame(Cmd::StorageReset, SEQ, &Ok::<(), RynkError>(()))
        ),
        ("GetLockStatus request ()", encode_frame(Cmd::GetLockStatus, SEQ, &())),
        (
            "GetLockStatus reply Ok(LockStatus{true,false,2,[(1,2),(3,4)]})",
            encode_frame(
                Cmd::GetLockStatus,
                SEQ,
                &Ok::<LockStatus, RynkError>(lock_status.clone())
            ),
        ),
        ("UnlockPoll request ()", encode_frame(Cmd::UnlockPoll, SEQ, &())),
        (
            "UnlockPoll reply Ok(LockStatus{true,false,2,[(1,2),(3,4)]})",
            encode_frame(Cmd::UnlockPoll, SEQ, &Ok::<LockStatus, RynkError>(lock_status.clone())),
        ),
        ("Lock request ()", encode_frame(Cmd::Lock, SEQ, &())),
        (
            "Lock reply Ok(())",
            encode_frame(Cmd::Lock, SEQ, &Ok::<(), RynkError>(()))
        ),
        ("GetLayout request 256", encode_frame(Cmd::GetLayout, SEQ, &256u32)),
        (
            "GetLayout reply Ok(LayoutChunk{300,[0x0a,0x0b,0x0c]})",
            encode_frame(Cmd::GetLayout, SEQ, &Ok::<LayoutChunk, RynkError>(ex.layout.clone())),
        ),
        ("GetDeviceInfo request ()", encode_frame(Cmd::GetDeviceInfo, SEQ, &())),
        (
            "GetDeviceInfo reply Ok(DeviceInfo{1.2.3,4,5,RMK,..})",
            encode_frame(
                Cmd::GetDeviceInfo,
                SEQ,
                &Ok::<DeviceInfo, RynkError>(ex.device_info.clone())
            ),
        ),
        // Keymap / encoder (0x01xx).
        (
            "GetKeyAction request KeyPosition{0,5,13}",
            encode_frame(Cmd::GetKeyAction, SEQ, &key_pos)
        ),
        (
            "GetKeyAction reply Ok(Morse(7))",
            encode_frame(Cmd::GetKeyAction, SEQ, &Ok::<KeyAction, RynkError>(KeyAction::Morse(7))),
        ),
        (
            "SetKeyAction request SetKeyRequest{{0,5,13},Morse(7)}",
            encode_frame(Cmd::SetKeyAction, SEQ, &set_key)
        ),
        (
            "SetKeyAction reply Ok(())",
            encode_frame(Cmd::SetKeyAction, SEQ, &Ok::<(), RynkError>(()))
        ),
        (
            "SetKeyAction reply Err(Invalid)",
            encode_frame(Cmd::SetKeyAction, SEQ, &Err::<(), RynkError>(RynkError::Invalid)),
        ),
        (
            "GetDefaultLayer request ()",
            encode_frame(Cmd::GetDefaultLayer, SEQ, &())
        ),
        (
            "GetDefaultLayer reply Ok(2)",
            encode_frame(Cmd::GetDefaultLayer, SEQ, &Ok::<u8, RynkError>(2)),
        ),
        (
            "SetDefaultLayer request 2",
            encode_frame(Cmd::SetDefaultLayer, SEQ, &2u8)
        ),
        (
            "SetDefaultLayer reply Ok(())",
            encode_frame(Cmd::SetDefaultLayer, SEQ, &Ok::<(), RynkError>(())),
        ),
        (
            "GetEncoderAction request GetEncoderRequest{1,2}",
            encode_frame(
                Cmd::GetEncoderAction,
                SEQ,
                &GetEncoderRequest {
                    encoder_id: 1,
                    layer: 2
                }
            ),
        ),
        (
            "GetEncoderAction reply Ok(EncoderAction{Morse(3),No})",
            encode_frame(Cmd::GetEncoderAction, SEQ, &Ok::<EncoderAction, RynkError>(ex.encoder)),
        ),
        (
            "SetEncoderAction request SetEncoderRequest{1,2,{Morse(3),No}}",
            encode_frame(
                Cmd::SetEncoderAction,
                SEQ,
                &SetEncoderRequest {
                    encoder_id: 1,
                    layer: 2,
                    action: ex.encoder
                },
            ),
        ),
        (
            "SetEncoderAction reply Ok(())",
            encode_frame(Cmd::SetEncoderAction, SEQ, &Ok::<(), RynkError>(())),
        ),
        // Macro (0x02xx).
        (
            "GetMacro request GetMacroRequest{256}",
            encode_frame(Cmd::GetMacro, SEQ, &GetMacroRequest { offset: 256 }),
        ),
        (
            "GetMacro reply Ok(MacroData{[0x01,0x02,0x03]})",
            encode_frame(Cmd::GetMacro, SEQ, &Ok::<MacroData, RynkError>(ex.macro_data.clone())),
        ),
        (
            "SetMacro request SetMacroRequest{2,[0x01,0x02,0x03]}",
            encode_frame(
                Cmd::SetMacro,
                SEQ,
                &SetMacroRequest {
                    offset: 2,
                    data: ex.macro_data.clone()
                },
            ),
        ),
        (
            "SetMacro reply Ok(())",
            encode_frame(Cmd::SetMacro, SEQ, &Ok::<(), RynkError>(()))
        ),
        // Combo (0x03xx).
        ("GetCombo request 3", encode_frame(Cmd::GetCombo, SEQ, &3u8)),
        (
            "GetCombo reply Ok(Combo{[Single(A)],Morse(1),L2})",
            encode_frame(Cmd::GetCombo, SEQ, &Ok::<Combo, RynkError>(ex.combo.clone())),
        ),
        (
            "SetCombo request SetComboRequest{3,combo}",
            encode_frame(
                Cmd::SetCombo,
                SEQ,
                &SetComboRequest {
                    index: 3,
                    config: ex.combo.clone()
                }
            ),
        ),
        (
            "SetCombo reply Ok(())",
            encode_frame(Cmd::SetCombo, SEQ, &Ok::<(), RynkError>(()))
        ),
        // Morse (0x04xx).
        ("GetMorse request 0", encode_frame(Cmd::GetMorse, SEQ, &0u8)),
        (
            "GetMorse reply Ok(Morse{TAP->Key(A)})",
            encode_frame(Cmd::GetMorse, SEQ, &Ok::<Morse, RynkError>(ex.morse.clone())),
        ),
        (
            "SetMorse request SetMorseRequest{0,morse}",
            encode_frame(
                Cmd::SetMorse,
                SEQ,
                &SetMorseRequest {
                    index: 0,
                    config: ex.morse.clone()
                }
            ),
        ),
        (
            "SetMorse reply Ok(())",
            encode_frame(Cmd::SetMorse, SEQ, &Ok::<(), RynkError>(()))
        ),
        // Fork (0x05xx).
        ("GetFork request 2", encode_frame(Cmd::GetFork, SEQ, &2u8)),
        (
            "GetFork reply Ok(Fork{Single(A),No,Morse(2)})",
            encode_frame(Cmd::GetFork, SEQ, &Ok::<Fork, RynkError>(ex.fork))
        ),
        (
            "SetFork request SetForkRequest{2,fork}",
            encode_frame(
                Cmd::SetFork,
                SEQ,
                &SetForkRequest {
                    index: 2,
                    config: ex.fork
                }
            ),
        ),
        (
            "SetFork reply Ok(())",
            encode_frame(Cmd::SetFork, SEQ, &Ok::<(), RynkError>(()))
        ),
        // Behavior (0x06xx).
        (
            "GetBehaviorConfig request ()",
            encode_frame(Cmd::GetBehaviorConfig, SEQ, &())
        ),
        (
            "GetBehaviorConfig reply Ok(BehaviorConfig{50..120})",
            encode_frame(
                Cmd::GetBehaviorConfig,
                SEQ,
                &Ok::<BehaviorConfig, RynkError>(ex.behavior)
            ),
        ),
        (
            "SetBehaviorConfig request BehaviorConfig{50..120}",
            encode_frame(Cmd::SetBehaviorConfig, SEQ, &ex.behavior)
        ),
        (
            "SetBehaviorConfig reply Ok(())",
            encode_frame(Cmd::SetBehaviorConfig, SEQ, &Ok::<(), RynkError>(())),
        ),
        // Connection (0x07xx).
        (
            "GetConnectionType request ()",
            encode_frame(Cmd::GetConnectionType, SEQ, &())
        ),
        (
            "GetConnectionType reply Ok(Ble)",
            encode_frame(
                Cmd::GetConnectionType,
                SEQ,
                &Ok::<ConnectionType, RynkError>(ConnectionType::Ble)
            ),
        ),
        (
            "GetConnectionStatus request ()",
            encode_frame(Cmd::GetConnectionStatus, SEQ, &())
        ),
        (
            "GetConnectionStatus reply Ok(ConnectionStatus{Configured,{1,Adv},Ble})",
            encode_frame(
                Cmd::GetConnectionStatus,
                SEQ,
                &Ok::<ConnectionStatus, RynkError>(ex.connection)
            ),
        ),
        // Status (0x08xx).
        (
            "GetCurrentLayer request ()",
            encode_frame(Cmd::GetCurrentLayer, SEQ, &())
        ),
        (
            "GetCurrentLayer reply Ok(1)",
            encode_frame(Cmd::GetCurrentLayer, SEQ, &Ok::<u8, RynkError>(1)),
        ),
        ("GetMatrixState request ()", encode_frame(Cmd::GetMatrixState, SEQ, &())),
        (
            "GetMatrixState reply Ok(MatrixState{[0x05,0x00,0x20]})",
            encode_frame(
                Cmd::GetMatrixState,
                SEQ,
                &Ok::<MatrixState, RynkError>(ex.matrix.clone())
            ),
        ),
        ("GetWpm request ()", encode_frame(Cmd::GetWpm, SEQ, &())),
        (
            "GetWpm reply Ok(42)",
            encode_frame(Cmd::GetWpm, SEQ, &Ok::<u16, RynkError>(42))
        ),
        ("GetSleepState request ()", encode_frame(Cmd::GetSleepState, SEQ, &())),
        (
            "GetSleepState reply Ok(true)",
            encode_frame(Cmd::GetSleepState, SEQ, &Ok::<bool, RynkError>(true)),
        ),
        (
            "GetLedIndicator request ()",
            encode_frame(Cmd::GetLedIndicator, SEQ, &())
        ),
        (
            "GetLedIndicator reply Ok(LedIndicator(Num|Scroll))",
            encode_frame(Cmd::GetLedIndicator, SEQ, &Ok::<LedIndicator, RynkError>(led)),
        ),
        // Connection / status rows behind `_ble` and `split`.
        ("GetBleStatus request ()", encode_frame(Cmd::GetBleStatus, SEQ, &())),
        (
            "GetBleStatus reply Ok(BleStatus{1,Advertising})",
            encode_frame(Cmd::GetBleStatus, SEQ, &Ok::<BleStatus, RynkError>(ex.connection.ble)),
        ),
        (
            "SwitchBleProfile request 1",
            encode_frame(Cmd::SwitchBleProfile, SEQ, &1u8)
        ),
        (
            "SwitchBleProfile reply Ok(())",
            encode_frame(Cmd::SwitchBleProfile, SEQ, &Ok::<(), RynkError>(())),
        ),
        (
            "ClearBleProfile request 1",
            encode_frame(Cmd::ClearBleProfile, SEQ, &1u8)
        ),
        (
            "ClearBleProfile reply Ok(())",
            encode_frame(Cmd::ClearBleProfile, SEQ, &Ok::<(), RynkError>(())),
        ),
        (
            "GetBatteryStatus request ()",
            encode_frame(Cmd::GetBatteryStatus, SEQ, &())
        ),
        (
            "GetBatteryStatus reply Ok(Available{Discharging,85})",
            encode_frame(Cmd::GetBatteryStatus, SEQ, &Ok::<BatteryStatus, RynkError>(ex.battery)),
        ),
        (
            "GetPeripheralStatus request 1",
            encode_frame(Cmd::GetPeripheralStatus, SEQ, &1u8)
        ),
        (
            "GetPeripheralStatus reply Ok(PeripheralStatus{true,Available{Discharging,85}})",
            encode_frame(
                Cmd::GetPeripheralStatus,
                SEQ,
                &Ok::<PeripheralStatus, RynkError>(PeripheralStatus {
                    connected: true,
                    battery: ex.battery,
                }),
            ),
        ),
        // Topics (0x80xx, server→host push, SEQ 0).
        ("LayerChange topic 3", encode_frame(Cmd::LayerChange, 0, &3u8)),
        ("WpmUpdate topic 42", encode_frame(Cmd::WpmUpdate, 0, &42u16)),
        (
            "ConnectionChange topic ConnectionStatus{Configured,{1,Adv},Ble}",
            encode_frame(Cmd::ConnectionChange, 0, &ex.connection)
        ),
        ("SleepState topic true", encode_frame(Cmd::SleepState, 0, &true)),
        (
            "LedIndicatorChange topic LedIndicator(Num|Scroll)",
            encode_frame(Cmd::LedIndicatorChange, 0, &led)
        ),
        (
            "BatteryStatusChange topic Available{Discharging,85}",
            encode_frame(Cmd::BatteryStatusChange, 0, &ex.battery)
        ),
    ];
    let view: alloc::vec::Vec<(&str, &[u8])> = entries.iter().map(|(l, b)| (*l, b.as_slice())).collect();

    let actual = snapshot::format_value_snapshot(
        "snapshots/wire_frames.snap",
        "Wire-format FRAME snapshot",
        "# Each entry is a full Rynk frame — a 3-byte header (CMD u16 LE + SEQ u8) + postcard\n\
         # payload, COBS-encoded with a trailing 0x00 delimiter — one per protocol message; the\n\
         # label names the decoded payload (`()` = empty). A diff means the header, a CMD number,\n\
         # or a message frame changed. If intentional, bump ProtocolVersion::CURRENT and regenerate:",
        "wire_frames",
        &view,
    );
    snapshot::assert_snapshot("snapshots/wire_frames.snap", actual);
}

/// The human-readable protocol reference under `docs/`, rendered from the
/// `ENDPOINT_META`/`TOPIC_META` tables. Those tables are not feature-gated, so
/// every feature set renders identical output; a diff fails CI (regenerate with
/// `UPDATE_SNAPSHOTS=1`), keeping the doc in lockstep with the wire contract.
mod protocol_reference {
    extern crate alloc;
    extern crate std;

    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;
    use std::path::PathBuf;

    use super::super::command::{ENDPOINT_META, EndpointMeta, TOPIC_META, TopicMeta};
    use super::ProtocolVersion;
    use super::snapshot::assert_snapshot_at;

    /// Repo-relative location of the generated page.
    const DOC_PATH: &str = "docs/docs/main/docs/development/rynk_protocol.md";

    /// Pull the doc text (Notes) and `cfg` feature out of a row's stringified
    /// attributes. `///` docs stringify as raw strings `#[doc = r"…"]`, wrapping
    /// after `doc =` when long — hence the whitespace skip and raw-delimiter scan.
    fn parse_attrs(attrs: &str) -> (String, Option<&str>) {
        let mut notes = String::new();
        let mut rest = attrs;
        while let Some(i) = rest.find("doc =") {
            rest = rest[i + 5..].trim_start();
            rest = rest.strip_prefix('r').unwrap_or(rest);
            let hashes = rest.len() - rest.trim_start_matches('#').len();
            rest = &rest[hashes..]; // now at the opening quote
            let close = format!("\"{}", "#".repeat(hashes));
            let Some(body) = rest.strip_prefix('"').and_then(|s| s.split(&close).next()) else {
                break;
            };
            if !notes.is_empty() {
                notes.push(' ');
            }
            notes.push_str(body.trim());
            rest = &rest[1 + body.len() + close.len()..];
        }
        // Rustdoc intra-links render as broken md links; keep just the code span.
        let notes = notes.replace("[`", "`").replace("`]", "`");
        let feature = attrs.find("feature = \"").and_then(|i| {
            let s = &attrs[i + 11..];
            s.find('"').map(|end| &s[..end])
        });
        (notes, feature)
    }

    /// Render `rows` as a column-aligned GFM table.
    fn table(header: &[&str], rows: &[Vec<String>]) -> String {
        let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
        for row in rows {
            for (w, cell) in widths.iter_mut().zip(row) {
                *w = (*w).max(cell.chars().count());
            }
        }
        let mut out = String::new();
        let emit = |out: &mut String, cells: &[String]| {
            out.push('|');
            for (w, cell) in widths.iter().zip(cells) {
                out.push_str(&format!(" {:w$} |", cell, w = w));
            }
            out.push('\n');
        };
        emit(&mut out, &header.iter().map(|h| String::from(*h)).collect::<Vec<_>>());
        emit(&mut out, &widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>());
        for row in rows {
            emit(&mut out, row);
        }
        out
    }

    fn endpoint_rows() -> Vec<Vec<String>> {
        ENDPOINT_META
            .iter()
            .map(
                |EndpointMeta {
                     name,
                     cmd,
                     request,
                     response,
                     attrs,
                 }| {
                    let (notes, feature) = parse_attrs(attrs);
                    let feature = feature.map(|f| format!("`{f}`")).unwrap_or_default();
                    alloc::vec![
                        format!("`0x{cmd:04X}`"),
                        format!("`{name}`"),
                        format!("`{request}`"),
                        format!("`{response}`"),
                        feature,
                        notes,
                    ]
                },
            )
            .collect()
    }

    fn topic_rows() -> Vec<Vec<String>> {
        TOPIC_META
            .iter()
            .map(
                |TopicMeta {
                     name,
                     cmd,
                     payload,
                     attrs,
                 }| {
                    let (notes, feature) = parse_attrs(attrs);
                    alloc::vec![
                        format!("`0x{cmd:04X}`"),
                        format!("`{name}`"),
                        format!("`{payload}`"),
                        feature.map(|f| format!("`{f}`")).unwrap_or_default(),
                        notes,
                    ]
                },
            )
            .collect()
    }

    fn render() -> String {
        let v = ProtocolVersion::CURRENT;
        format!(
            "{header}\n\n\
             # Rynk Protocol Reference\n\n\
             Current protocol version: **{major}.{minor}**.\n\n\
             Every transport (USB CDC, BLE GATT, BLE HID) carries the same frame — a 3-byte header plus a [postcard](https://docs.rs/postcard)-encoded payload:\n\n\
             ```text\n\
             ┌──────────────┬───────────┐\n\
             │  CMD u16 LE  │  SEQ u8   │  ← 3-byte header\n\
             ├──────────────┴───────────┤\n\
             │ postcard-encoded payload │\n\
             └──────────────────────────┘\n\
             ```\n\n\
             On the wire the whole frame is COBS-encoded and terminated by a single `0x00` delimiter, so the byte stream is self-synchronizing.\n\n\
             - **Requests** use CMD `0x0000..=0x7FFF`. The response echoes CMD and SEQ and wraps its payload in postcard `Result<T, RynkError>` (`T = ()` for `Set*`).\n\
             - **Topics** use CMD `0x8000..=0xFFFF` (server → host push, SEQ `0`, bare payload).\n\n\
             Which commands a firmware answers depends on the RMK Cargo features it was built with: a row with no **Feature** is present once `rynk` is on, and the rest need their feature (`_ble`, `split`, …) compiled in. A command the firmware wasn't built with answers `UnknownCmd`.\n\n\
             ## Endpoints\n\n\
             {endpoints}\n\
             ## Topics\n\n\
             Topics are best-effort pushes; the `Get*` endpoints above mirror their payloads so a host can recover a missed push.\n\n\
             {topics}",
            header = "<!-- GENERATED — do not edit. Rendered from the `endpoints!`/`topics!` tables in\n     rmk-types/src/protocol/rynk/command.rs. Regenerate with:\n     UPDATE_SNAPSHOTS=1 cargo test -p rmk-types --features rynk protocol_reference -->",
            major = v.major,
            minor = v.minor,
            endpoints = table(
                &["CMD", "Name", "Request", "Response", "Feature", "Notes"],
                &endpoint_rows()
            ),
            topics = table(&["CMD", "Name", "Payload", "Feature", "Notes"], &topic_rows()),
        )
    }

    #[test]
    fn protocol_reference_is_current() {
        // rmk-types/../ is the repo root.
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(DOC_PATH);
        assert_snapshot_at(path, render());
    }
}
