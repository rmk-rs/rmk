//! Wasm-facing Vial client handle.
//!
//! Same lifecycle as [`crate::client`]: JS owns the byte link and hands it to
//! [`connect_vial`], which probes the device, pulls and decodes `vial.json`,
//! and returns a [`VialClient`] whose method surface mirrors `RynkClient` —
//! the GUI's store drives either through one structural interface. Vial has no
//! server pushes, so `next_topic` parks on the session pump and rejects when
//! the link dies, which is all the store's topic loop needs.
//!
//! What the protocol cannot express rejects as `Rejected(Unimplemented)`
//! (`reboot`, default layer, BLE, forks), and the capabilities are synthesized
//! so those paths are gated off in the UI to begin with.

use std::cell::RefCell;

use rynk::rmk_types::action::{EncoderAction, KeyAction};
use rynk::rmk_types::battery::BatteryStatus;
use rynk::rmk_types::ble::BleStatus;
use rynk::rmk_types::combo::Combo;
use rynk::rmk_types::connection::{ConnectionStatus, ConnectionType, UsbState};
use rynk::rmk_types::constants::MACRO_DATA_SIZE;
use rynk::rmk_types::fork::Fork;
use rynk::rmk_types::led_indicator::LedIndicator;
use rynk::rmk_types::morse::Morse;
use rynk::rmk_types::protocol::rynk::{
    BehaviorConfig, DEVICE_INFO_STRING_SIZE, DeviceCapabilities, DeviceInfo, FirmwareVersion, LockStatus, MacroData,
    MatrixState, PeripheralStatus, ProtocolVersion, RynkError, StorageResetMode,
};
use rynk::rmk_types::protocol::vial::SettingKey;
use rynk::vial::VialSession;
use rynk::{LayoutInfo, RynkDevice, RynkHostError, TopicEvent};
use wasm_bindgen::prelude::*;

use crate::transport::{JsByteLink, WasmReader, WasmWriter};

/// One `BehaviorConfig` field: its Vial setting key and a projection to it.
type BehaviorField = (SettingKey, fn(&mut BehaviorConfig) -> &mut u16);

/// The BehaviorConfig fields and the Vial setting keys they live under.
const BEHAVIOR_KEYS: [BehaviorField; 4] = [
    (SettingKey::ComboTimeout, |b| &mut b.combo_timeout_ms),
    (SettingKey::OneShotTimeout, |b| &mut b.oneshot_timeout_ms),
    (SettingKey::TapInterval, |b| &mut b.tap_interval_ms),
    (SettingKey::TapCapslockInterval, |b| &mut b.tap_capslock_interval_ms),
];

fn unimplemented_cmd() -> JsValue {
    RynkHostError::Rejected(RynkError::Unimplemented).into()
}

fn hstr(s: &str) -> heapless::String<DEVICE_INFO_STRING_SIZE> {
    let mut out = heapless::String::new();
    for c in s.chars() {
        if out.push(c).is_err() {
            break;
        }
    }
    out
}

fn hex_or_int(value: Option<&serde_json::Value>) -> u16 {
    match value {
        Some(serde_json::Value::String(s)) => {
            u16::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).unwrap_or(0)
        }
        Some(v) => v.as_u64().unwrap_or(0) as u16,
        None => 0,
    }
}

/// `vial.json` payloads are xz in RMK builds and either xz or lzma-alone from
/// vial-qmk's Python packer; the magic bytes tell them apart.
fn decompress(def: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    if def.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        lzma_rs::xz_decompress(&mut &def[..], &mut out).map_err(|e| format!("xz: {e:?}"))?;
    } else {
        lzma_rs::lzma_decompress(&mut &def[..], &mut out).map_err(|e| format!("lzma: {e:?}"))?;
    }
    Ok(out)
}

/// Live Vial client handle exposed to JavaScript. Everything the definition
/// answers (layout, capabilities, identity) is decoded once at connect; the
/// methods only reach for the wire when the device holds the data.
#[wasm_bindgen]
pub struct VialClient {
    session: VialSession<WasmReader, WasmWriter>,
    caps: DeviceCapabilities,
    layout: LayoutInfo,
    info: DeviceInfo,
    version: ProtocolVersion,
    /// Challenge positions from the last `GetUnlockStatus`, for the ceremony's
    /// `LockStatus.key_positions` while polling.
    unlock_keys: RefCell<Vec<(u8, u8)>>,
    /// An `UnlockStart` has been sent and the window not yet resolved.
    unlock_armed: RefCell<bool>,
}

/// Handshake over an already-open JS link, fetch + decode the keyboard
/// definition, and return a client. Rejects with `Rejected(NotReady)` when a
/// dongle relay has no keyboard behind it.
#[wasm_bindgen]
pub async fn connect_vial(link: JsByteLink) -> Result<VialClient, JsValue> {
    let (reader, writer) = link.open().await?;
    let session = VialSession::new(reader, writer);

    session.via_protocol_version().await?;
    let (vial_version, _uid) = session.vial_keyboard_id().await?;
    let def = session.keyboard_def().await?;
    let json: serde_json::Value = decompress(&def)
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
        .map_err(|e| RynkHostError::Layout(format!("vial.json: {e}")))?;

    let generated = rynk_kle::convert_kle(&json).map_err(RynkHostError::Layout)?;
    let layout = rynk_kle::decode_layout(&generated.layout_toml).map_err(RynkHostError::Layout)?;

    let matrix = |key: &str| {
        json.pointer(&format!("/matrix/{key}"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8
    };
    let (num_rows, num_cols) = (matrix("rows"), matrix("cols"));
    if num_rows == 0 || num_cols == 0 {
        return Err(RynkHostError::Layout("vial.json has no matrix.rows/cols".into()).into());
    }

    let num_layers = session.layer_count().await?;
    let macro_space_size = session.macro_buffer_size().await?;
    let (max_morse, max_combos, _key_overrides) = session.dynamic_entry_counts().await?;
    let (_, _, unlock_keys) = session.unlock_status().await?;

    let num_encoders = layout.variants.iter().map(|v| v.encoders.len()).max().unwrap_or(0) as u8;

    let caps = DeviceCapabilities {
        num_layers,
        num_rows,
        num_cols,
        num_encoders,
        max_combos,
        max_combo_keys: 4,
        macro_space_size,
        max_morse,
        max_patterns_per_key: 4,
        max_forks: 0,
        storage_enabled: true,
        lighting_enabled: false,
        is_split: false,
        num_split_peripherals: 0,
        ble_enabled: false,
        num_ble_profiles: 0,
        max_payload_size: 32,
        max_bulk_keys: 14,
        max_bulk_items: 1,
        macro_chunk_size: 28,
        bulk_transfer_supported: false,
    };
    let info = DeviceInfo {
        rmk_version: FirmwareVersion {
            major: 0,
            minor: 0,
            patch: 0,
        },
        vendor_id: hex_or_int(json.get("vendorId")),
        product_id: hex_or_int(json.get("productId")),
        manufacturer: hstr(""),
        product_name: hstr(json.get("name").and_then(|v| v.as_str()).unwrap_or("")),
        serial_number: hstr(""),
    };

    Ok(VialClient {
        session,
        caps,
        layout,
        info,
        version: ProtocolVersion {
            major: 0,
            minor: vial_version.min(u8::MAX as u32) as u8,
        },
        unlock_keys: RefCell::new(unlock_keys),
        unlock_armed: RefCell::new(false),
    })
}

impl VialClient {
    fn lock_status_from(&self, unlocked: bool, unlocking: bool, remaining: u8) -> LockStatus {
        let cached = self.unlock_keys.borrow();
        let mut key_positions = heapless::Vec::new();
        for pos in cached.iter().take(key_positions.capacity()) {
            let _ = key_positions.push(*pos);
        }
        LockStatus {
            locked: !unlocked,
            unlocking,
            remaining_keys: if unlocking { remaining } else { cached.len() as u8 },
            key_positions,
        }
    }
}

#[wasm_bindgen]
impl VialClient {
    /// Vial has no topic pushes: parks until the link dies, then rejects —
    /// exactly the contract the store's topic loop and teardown rely on.
    pub async fn next_topic(&self) -> Result<TopicEvent, JsValue> {
        Err(self.session.run_until_disconnect().await.into())
    }

    // -- system --

    pub async fn get_version(&self) -> Result<ProtocolVersion, JsValue> {
        Ok(self.version)
    }

    pub async fn get_capabilities(&self) -> Result<DeviceCapabilities, JsValue> {
        Ok(self.caps)
    }

    pub async fn get_device_info(&self) -> Result<DeviceInfo, JsValue> {
        Ok(self.info.clone())
    }

    pub async fn reboot(&self) -> Result<(), JsValue> {
        Err(unimplemented_cmd())
    }

    pub async fn bootloader_jump(&self) -> Result<(), JsValue> {
        Ok(self.session.bootloader_jump().await?)
    }

    pub async fn storage_reset(&self, mode: StorageResetMode) -> Result<(), JsValue> {
        match mode {
            StorageResetMode::Full => Ok(self.session.eeprom_reset().await?),
            _ => Err(unimplemented_cmd()),
        }
    }

    // -- lock gate --

    pub async fn get_lock_status(&self) -> Result<LockStatus, JsValue> {
        let (unlocked, unlocking, keys) = self.session.unlock_status().await?;
        *self.unlock_keys.borrow_mut() = keys;
        if unlocked {
            *self.unlock_armed.borrow_mut() = false;
        }
        Ok(self.lock_status_from(unlocked, unlocking, 0))
    }

    pub async fn unlock_poll(&self) -> Result<LockStatus, JsValue> {
        let armed = *self.unlock_armed.borrow();
        if !armed {
            self.session.unlock_start().await?;
            *self.unlock_armed.borrow_mut() = true;
        }
        let (unlocked, unlocking, remaining) = self.session.unlock_poll().await?;
        if unlocked {
            *self.unlock_armed.borrow_mut() = false;
        }
        Ok(self.lock_status_from(unlocked, unlocking, remaining))
    }

    pub async fn lock(&self) -> Result<(), JsValue> {
        *self.unlock_armed.borrow_mut() = false;
        Ok(self.session.lock().await?)
    }

    // -- keymap --

    pub async fn get_key(&self, layer: u8, row: u8, col: u8) -> Result<KeyAction, JsValue> {
        let flat =
            (layer as usize * self.caps.num_rows as usize + row as usize) * self.caps.num_cols as usize + col as usize;
        let page = self.session.keymap_page(flat as u16 * 2, 2).await?;
        Ok(page.into_iter().next().unwrap_or(KeyAction::No))
    }

    pub async fn set_key(&self, layer: u8, row: u8, col: u8, action: KeyAction) -> Result<(), JsValue> {
        Ok(self.session.set_keycode(layer, row, col, action).await?)
    }

    pub async fn get_default_layer(&self) -> Result<u8, JsValue> {
        Ok(0)
    }

    /// Vial has no default-layer command. Layer 0 — the value every Vial
    /// backup round-trips — is already the truth, so only a change rejects;
    /// this keeps config restores from failing on a no-op.
    pub async fn set_default_layer(&self, layer: u8) -> Result<(), JsValue> {
        if layer == 0 { Ok(()) } else { Err(unimplemented_cmd()) }
    }

    pub async fn read_all_keymap(&self) -> Result<Vec<KeyAction>, JsValue> {
        Ok(self
            .session
            .read_all_keymap(self.caps.num_layers, self.caps.num_rows, self.caps.num_cols)
            .await?)
    }

    pub async fn write_all_keymap(&self, actions: Vec<KeyAction>) -> Result<(), JsValue> {
        Ok(self
            .session
            .write_all_keymap(&actions, self.caps.num_rows, self.caps.num_cols)
            .await?)
    }

    pub async fn get_encoder(&self, encoder_id: u8, layer: u8) -> Result<EncoderAction, JsValue> {
        Ok(self.session.encoder(encoder_id, layer).await?)
    }

    pub async fn set_encoder(&self, encoder_id: u8, layer: u8, action: EncoderAction) -> Result<(), JsValue> {
        Ok(self.session.set_encoder(encoder_id, layer, action).await?)
    }

    pub async fn get_layout(&self) -> Result<LayoutInfo, JsValue> {
        Ok(self.layout.clone())
    }

    // -- combos / morses / forks / macros --

    pub async fn read_all_combos(&self) -> Result<Vec<Combo>, JsValue> {
        let mut combos = Vec::with_capacity(self.caps.max_combos as usize);
        for index in 0..self.caps.max_combos {
            combos.push(self.session.combo(index).await?);
        }
        Ok(combos)
    }

    pub async fn write_all_combos(&self, configs: Vec<Combo>) -> Result<(), JsValue> {
        for (index, combo) in configs.iter().enumerate() {
            self.session.set_combo(index as u8, combo).await?;
        }
        Ok(())
    }

    pub async fn get_combo(&self, index: u8) -> Result<Combo, JsValue> {
        Ok(self.session.combo(index).await?)
    }

    pub async fn set_combo(&self, index: u8, config: Combo) -> Result<(), JsValue> {
        Ok(self.session.set_combo(index, &config).await?)
    }

    pub async fn read_all_morses(&self) -> Result<Vec<Morse>, JsValue> {
        let mut morses = Vec::with_capacity(self.caps.max_morse as usize);
        for index in 0..self.caps.max_morse {
            morses.push(self.session.morse(index).await?);
        }
        Ok(morses)
    }

    pub async fn write_all_morses(&self, configs: Vec<Morse>) -> Result<(), JsValue> {
        for (index, morse) in configs.iter().enumerate() {
            self.session.set_morse(index as u8, morse).await?;
        }
        Ok(())
    }

    pub async fn get_morse(&self, index: u8) -> Result<Morse, JsValue> {
        Ok(self.session.morse(index).await?)
    }

    pub async fn set_morse(&self, index: u8, config: Morse) -> Result<(), JsValue> {
        Ok(self.session.set_morse(index, &config).await?)
    }

    /// No key-override relay on the Vial wire; `max_forks` is 0 so nothing asks.
    pub async fn get_fork(&self, _index: u8) -> Result<Fork, JsValue> {
        Err(unimplemented_cmd())
    }

    pub async fn set_fork(&self, _index: u8, _config: Fork) -> Result<(), JsValue> {
        Err(unimplemented_cmd())
    }

    pub async fn get_macro(&self, offset: u16) -> Result<MacroData, JsValue> {
        let remaining = self.caps.macro_space_size.saturating_sub(offset);
        let len = remaining.min(self.caps.macro_chunk_size).min(MACRO_DATA_SIZE as u16) as u8;
        let mut data = heapless::Vec::new();
        if len > 0 {
            for byte in self.session.macro_buffer_page(offset, len).await? {
                let _ = data.push(byte);
            }
        }
        Ok(MacroData { data })
    }

    pub async fn set_macro(&self, offset: u16, data: MacroData) -> Result<(), JsValue> {
        Ok(self.session.write_macro_buffer(offset, &data.data).await?)
    }

    // -- behavior --

    pub async fn get_behavior(&self) -> Result<BehaviorConfig, JsValue> {
        let mut behavior = BehaviorConfig {
            combo_timeout_ms: 0,
            oneshot_timeout_ms: 0,
            tap_interval_ms: 0,
            tap_capslock_interval_ms: 0,
        };
        for (key, field) in BEHAVIOR_KEYS {
            if let Some(value) = self.session.behavior_setting(key as u16).await? {
                *field(&mut behavior) = value;
            }
        }
        Ok(behavior)
    }

    /// Writes all four keys unconditionally: the firmware treats an unknown
    /// setting key as a no-op, so a board without behavior settings absorbs
    /// this harmlessly instead of failing a config restore.
    pub async fn set_behavior(&self, config: BehaviorConfig) -> Result<(), JsValue> {
        let mut config = config;
        for (key, field) in BEHAVIOR_KEYS {
            self.session
                .set_behavior_setting(key as u16, *field(&mut config))
                .await?;
        }
        Ok(())
    }

    // -- status --

    pub async fn get_current_layer(&self) -> Result<u8, JsValue> {
        Ok(0)
    }

    pub async fn get_matrix_state(&self) -> Result<MatrixState, JsValue> {
        let bytes = self
            .session
            .matrix_state(self.caps.num_rows, self.caps.num_cols)
            .await?;
        let mut pressed_bitmap = heapless::Vec::new();
        for byte in bytes.into_iter().take(pressed_bitmap.capacity()) {
            let _ = pressed_bitmap.push(byte);
        }
        Ok(MatrixState { pressed_bitmap })
    }

    pub async fn get_battery_status(&self) -> Result<BatteryStatus, JsValue> {
        Ok(BatteryStatus::Unavailable)
    }

    pub async fn get_led_indicator(&self) -> Result<LedIndicator, JsValue> {
        Ok(LedIndicator::default())
    }

    pub async fn get_peripheral_status(&self, _slot: u8) -> Result<PeripheralStatus, JsValue> {
        Err(unimplemented_cmd())
    }

    pub async fn get_wpm(&self) -> Result<u16, JsValue> {
        Ok(0)
    }

    pub async fn get_sleep_state(&self) -> Result<bool, JsValue> {
        Ok(false)
    }

    // -- connection --

    pub async fn get_connection_type(&self) -> Result<ConnectionType, JsValue> {
        Ok(ConnectionType::Usb)
    }

    pub async fn get_connection_status(&self) -> Result<ConnectionStatus, JsValue> {
        Ok(ConnectionStatus {
            usb: UsbState::Configured,
            ..ConnectionStatus::new()
        })
    }

    pub async fn get_ble_status(&self) -> Result<BleStatus, JsValue> {
        Err(unimplemented_cmd())
    }

    pub async fn switch_ble_profile(&self, _slot: u8) -> Result<(), JsValue> {
        Err(unimplemented_cmd())
    }

    pub async fn clear_ble_profile(&self, _slot: u8) -> Result<(), JsValue> {
        Err(unimplemented_cmd())
    }
}
