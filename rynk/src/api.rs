//! Typed request methods for each protocol endpoint, built on top of the
//! driver core in `driver.rs`.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "alloc")]
use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "alloc")]
use embassy_futures::join::join_array;
#[cfg(feature = "alloc")]
use postcard::experimental::max_size::MaxSize;
#[cfg(feature = "alloc")]
use postcard::experimental::serialized_size;
use rmk_types::action::{EncoderAction, KeyAction};
use rmk_types::battery::BatteryStatus;
use rmk_types::ble::BleStatus;
use rmk_types::combo::Combo;
use rmk_types::connection::{ConnectionStatus, ConnectionType};
use rmk_types::fork::Fork;
use rmk_types::led_indicator::LedIndicator;
use rmk_types::morse::Morse;
use rmk_types::protocol::rynk::{
    BehaviorConfig, Cmd, DeviceCapabilities, DeviceInfo, GetComboBulkRequest, GetComboBulkResponse, GetEncoderRequest,
    GetKeymapBulkRequest, GetKeymapBulkResponse, GetMacroRequest, GetMorseBulkRequest, GetMorseBulkResponse,
    KeyPosition, LockStatus, MacroData, MatrixState, PeripheralStatus, ProtocolVersion, SetComboBulkRequest,
    SetComboRequest, SetEncoderRequest, SetForkRequest, SetKeyRequest, SetKeymapBulkRequest, SetMacroRequest,
    SetMorseBulkRequest, SetMorseRequest, StorageResetMode, command,
};
#[cfg(feature = "alloc")]
use rmk_types::protocol::rynk::{RYNK_HEADER_SIZE, RynkError, max_wire_size};
#[cfg(feature = "alloc")]
use serde::Serialize;

#[cfg(feature = "alloc")]
use crate::driver::MAX_IN_FLIGHT;
use crate::driver::{Client, RynkHostError};
#[cfg(feature = "alloc")]
use crate::layout::LayoutInfo;

/// A pipelined request parks in the firmware's shared frame buffer while another is served,
/// shrinking the reply window and thus the page it can return. This value is the worst parked size.
///
/// [`Client::read_all`] reads concurrently, so it discounts the parked bytes when sizing each
/// lane's window: the window then matches the largest page the firmware can still return with
/// the other lanes' requests parked, so one round trip fills it.
#[cfg(feature = "alloc")]
const PARKED_REQUEST_BYTES: usize = max_wire_size(RYNK_HEADER_SIZE + GetKeymapBulkRequest::POSTCARD_MAX_SIZE);

impl Client {
    fn require_bulk_transfer(&self, cmd: Cmd) -> Result<(), RynkHostError> {
        if self.capabilities.bulk_transfer_supported {
            Ok(())
        } else {
            Err(RynkHostError::Unsupported(cmd, "bulk transfer not supported"))
        }
    }

    fn require_ble(&self, cmd: Cmd) -> Result<(), RynkHostError> {
        if self.capabilities.ble_enabled {
            Ok(())
        } else {
            Err(RynkHostError::Unsupported(cmd, "BLE not enabled"))
        }
    }

    /// Read the firmware's protocol version.
    pub async fn get_version(&self) -> Result<ProtocolVersion, RynkHostError> {
        self.request::<command::GetVersion>(&()).await
    }

    /// Return the capability set saved during the connect handshake.
    /// Capabilities are firmware constants, so nothing is sent to the device.
    pub async fn get_capabilities(&self) -> Result<DeviceCapabilities, RynkHostError> {
        Ok(self.capabilities)
    }

    /// Read the firmware and device identity.
    pub async fn get_device_info(&self) -> Result<DeviceInfo, RynkHostError> {
        self.request::<command::GetDeviceInfo>(&()).await
    }

    /// Reboot the device. The firmware resets before it can reply, so `Ok(())` only
    /// means the request was queued — keep the driver running long enough to write it.
    pub async fn reboot(&self) -> Result<(), RynkHostError> {
        self.send_no_reply::<command::Reboot>(&()).await
    }

    /// Jump to the bootloader (DFU mode). No reply, exactly like
    /// [`reboot`](Self::reboot).
    pub async fn bootloader_jump(&self) -> Result<(), RynkHostError> {
        self.send_no_reply::<command::BootloaderJump>(&()).await
    }

    /// Reset persistent storage. Requires [`DeviceCapabilities::storage_enabled`]:
    /// without storage the wipe would silently do nothing, so nothing is sent.
    pub async fn storage_reset(&self, mode: StorageResetMode) -> Result<(), RynkHostError> {
        if !self.capabilities.storage_enabled {
            return Err(RynkHostError::Unsupported(Cmd::StorageReset, "storage not enabled"));
        }
        self.request::<command::StorageReset>(&mode).await
    }

    /// Read the current lock state; unlike [`unlock_poll`](Self::unlock_poll) this has
    /// no side effects. [`LockStatus::key_positions`] lists the keys to hold to unlock;
    /// empty while [`locked`](LockStatus::locked) means the device can never be
    /// unlocked (no `unlock_keys` in keyboard.toml).
    pub async fn get_lock_status(&self) -> Result<LockStatus, RynkHostError> {
        self.request::<command::GetLockStatus>(&()).await
    }

    /// Start or keep alive an unlock attempt, reporting which challenge keys are held
    /// right now. Call every ~150 ms while the user holds the keys from
    /// [`LockStatus::key_positions`]: [`remaining_keys`](LockStatus::remaining_keys)
    /// counts down and [`locked`](LockStatus::locked) turns false once all are held at
    /// once. The attempt expires ~500 ms after the last call, so to cancel it, just
    /// stop calling.
    pub async fn unlock_poll(&self) -> Result<LockStatus, RynkHostError> {
        self.request::<command::UnlockPoll>(&()).await
    }

    /// Lock the device again immediately. Does nothing on an `insecure`
    /// device.
    pub async fn lock(&self) -> Result<(), RynkHostError> {
        self.request::<command::Lock>(&()).await
    }

    /// Read one key's action.
    pub async fn get_key(&self, layer: u8, row: u8, col: u8) -> Result<KeyAction, RynkHostError> {
        self.request::<command::GetKeyAction>(&KeyPosition { layer, row, col })
            .await
    }

    /// Write one key's action.
    pub async fn set_key(&self, layer: u8, row: u8, col: u8, action: KeyAction) -> Result<(), RynkHostError> {
        let req = SetKeyRequest {
            position: KeyPosition { layer, row, col },
            action,
        };
        self.request::<command::SetKeyAction>(&req).await
    }

    /// Read the currently selected default layer index.
    pub async fn get_default_layer(&self) -> Result<u8, RynkHostError> {
        self.request::<command::GetDefaultLayer>(&()).await
    }

    /// Set the default layer.
    pub async fn set_default_layer(&self, layer: u8) -> Result<(), RynkHostError> {
        self.request::<command::SetDefaultLayer>(&layer).await
    }

    /// Read both rotation actions for one encoder on one layer.
    pub async fn get_encoder(&self, encoder_id: u8, layer: u8) -> Result<EncoderAction, RynkHostError> {
        self.request::<command::GetEncoderAction>(&GetEncoderRequest { encoder_id, layer })
            .await
    }

    /// Set both rotation actions for one encoder on one layer.
    pub async fn set_encoder(&self, encoder_id: u8, layer: u8, action: EncoderAction) -> Result<(), RynkHostError> {
        let req = SetEncoderRequest {
            encoder_id,
            layer,
            action,
        };
        self.request::<command::SetEncoderAction>(&req).await
    }

    /// Read one page of key actions starting at `(layer, start_row, start_col)`, walking
    /// column by column, then row by row, then layer by layer. A page holds up to
    /// `max_bulk_keys` actions, the last possibly fewer; a start position outside the
    /// keymap fails with `RynkError::Invalid`.
    /// Requires [`DeviceCapabilities::bulk_transfer_supported`]; nothing is sent otherwise.
    pub async fn get_keymap_bulk(
        &self,
        layer: u8,
        start_row: u8,
        start_col: u8,
    ) -> Result<GetKeymapBulkResponse, RynkHostError> {
        self.require_bulk_transfer(Cmd::GetKeymapBulk)?;
        self.request::<command::GetKeymapBulk>(&GetKeymapBulkRequest {
            layer,
            start_row,
            start_col,
        })
        .await
    }

    /// Write `request.actions` into the keymap starting at
    /// `(request.layer, request.start_row, request.start_col)`, walking the
    /// same order as [`get_keymap_bulk`](Self::get_keymap_bulk).
    /// Requires [`DeviceCapabilities::bulk_transfer_supported`]; nothing is sent otherwise.
    pub async fn set_keymap_bulk(&self, request: SetKeymapBulkRequest) -> Result<(), RynkHostError> {
        self.require_bulk_transfer(Cmd::SetKeymapBulk)?;
        self.request::<command::SetKeymapBulk>(&request).await
    }

    /// Read the physical layout, which the firmware serves as one compressed blob in
    /// pages. A firmware built without a `[layout].map` serves an empty blob and yields
    /// an empty [`LayoutInfo`], not an error.
    #[cfg(feature = "alloc")]
    pub async fn get_layout(&self) -> Result<LayoutInfo, RynkHostError> {
        const MAX_LAYOUT_BLOB_LEN: usize = 64 * 1024;
        let first = self.request::<command::GetLayout>(&0u32).await?;
        let total_len = first.total_len as usize;
        if total_len > MAX_LAYOUT_BLOB_LEN {
            return Err(RynkHostError::Layout(alloc::format!(
                "advertised layout blob length {total_len} exceeds maximum {MAX_LAYOUT_BLOB_LEN}"
            )));
        }
        // The first page fixes the page size, which is all [`Self::read_all`] needs to
        // read the rest on lanes — one round trip per `MAX_IN_FLIGHT` pages instead of
        // one each. It re-reads page 0, which rides an existing lane and costs nothing.
        let page = first.bytes.len();
        let mut blob: Vec<u8> = if page == 0 || total_len <= page {
            first.bytes.to_vec()
        } else {
            self.read_all(total_len, page, async |c, offset| {
                Ok(c.request::<command::GetLayout>(&(offset as u32)).await?.bytes.to_vec())
            })
            .await?
        };
        blob.truncate(total_len);
        LayoutInfo::from_compressed_blob(&blob).map_err(RynkHostError::Layout)
    }

    /// Read one combo entry by index.
    pub async fn get_combo(&self, index: u8) -> Result<Combo, RynkHostError> {
        self.request::<command::GetCombo>(&index).await
    }

    /// Write one combo entry by index.
    pub async fn set_combo(&self, index: u8, config: Combo) -> Result<(), RynkHostError> {
        self.request::<command::SetCombo>(&SetComboRequest { index, config })
            .await
    }

    /// Read one page of combos starting at slot `start_index`. A page holds
    /// up to `max_bulk_items` combos; the last one may hold fewer, and a
    /// `start_index` past the last slot returns an empty page.
    /// Requires [`DeviceCapabilities::bulk_transfer_supported`]; nothing is sent otherwise.
    pub async fn get_combo_bulk(&self, start_index: u8) -> Result<GetComboBulkResponse, RynkHostError> {
        self.require_bulk_transfer(Cmd::GetComboBulk)?;
        self.request::<command::GetComboBulk>(&GetComboBulkRequest { start_index })
            .await
    }

    /// Write `request.configs` into consecutive combo slots starting at
    /// `request.start_index`.
    /// Requires [`DeviceCapabilities::bulk_transfer_supported`]; nothing is sent otherwise.
    pub async fn set_combo_bulk(&self, request: SetComboBulkRequest) -> Result<(), RynkHostError> {
        self.require_bulk_transfer(Cmd::SetComboBulk)?;
        self.request::<command::SetComboBulk>(&request).await
    }

    /// Read one fork entry by index.
    pub async fn get_fork(&self, index: u8) -> Result<Fork, RynkHostError> {
        self.request::<command::GetFork>(&index).await
    }

    /// Write one fork entry by index.
    pub async fn set_fork(&self, index: u8, config: Fork) -> Result<(), RynkHostError> {
        self.request::<command::SetFork>(&SetForkRequest { index, config })
            .await
    }

    /// Read one morse entry by index.
    pub async fn get_morse(&self, index: u8) -> Result<Morse, RynkHostError> {
        self.request::<command::GetMorse>(&index).await
    }

    /// Write one morse entry by index.
    pub async fn set_morse(&self, index: u8, config: Morse) -> Result<(), RynkHostError> {
        self.request::<command::SetMorse>(&SetMorseRequest { index, config })
            .await
    }

    /// Read one page of morses starting at slot `start_index`. A page holds
    /// up to `max_bulk_items` morses; the last one may hold fewer, and a
    /// `start_index` past the last slot returns an empty page.
    /// Requires [`DeviceCapabilities::bulk_transfer_supported`]; nothing is sent otherwise.
    pub async fn get_morse_bulk(&self, start_index: u8) -> Result<GetMorseBulkResponse, RynkHostError> {
        self.require_bulk_transfer(Cmd::GetMorseBulk)?;
        self.request::<command::GetMorseBulk>(&GetMorseBulkRequest { start_index })
            .await
    }

    /// Write `request.configs` into consecutive morse slots starting at
    /// `request.start_index`.
    /// Requires [`DeviceCapabilities::bulk_transfer_supported`]; nothing is sent otherwise.
    pub async fn set_morse_bulk(&self, request: SetMorseBulkRequest) -> Result<(), RynkHostError> {
        self.require_bulk_transfer(Cmd::SetMorseBulk)?;
        self.request::<command::SetMorseBulk>(&request).await
    }

    /// Read one chunk of macro data starting at byte `offset`. Chunks are always full
    /// size, zero-filled past the end of macro space, so find the end by parsing the
    /// macro encoding rather than waiting for a short chunk.
    pub async fn get_macro(&self, offset: u16) -> Result<MacroData, RynkHostError> {
        self.request::<command::GetMacro>(&GetMacroRequest { offset }).await
    }

    /// Write one chunk of macro data starting at byte `offset`. Writes past
    /// the end of the device's macro space are truncated by the firmware.
    pub async fn set_macro(&self, offset: u16, data: MacroData) -> Result<(), RynkHostError> {
        self.request::<command::SetMacro>(&SetMacroRequest { offset, data })
            .await
    }

    /// Read the global behavior config.
    pub async fn get_behavior(&self) -> Result<BehaviorConfig, RynkHostError> {
        self.request::<command::GetBehaviorConfig>(&()).await
    }

    /// Write the global behavior config.
    pub async fn set_behavior(&self, config: BehaviorConfig) -> Result<(), RynkHostError> {
        self.request::<command::SetBehaviorConfig>(&config).await
    }

    /// Read the currently active layer.
    pub async fn get_current_layer(&self) -> Result<u8, RynkHostError> {
        self.request::<command::GetCurrentLayer>(&()).await
    }

    /// Read the matrix state: a bitmap of which keys are physically pressed.
    pub async fn get_matrix_state(&self) -> Result<MatrixState, RynkHostError> {
        self.request::<command::GetMatrixState>(&()).await
    }

    /// Read battery status. Requires [`DeviceCapabilities::ble_enabled`]; nothing is sent otherwise.
    pub async fn get_battery_status(&self) -> Result<BatteryStatus, RynkHostError> {
        self.require_ble(Cmd::GetBatteryStatus)?;
        self.request::<command::GetBatteryStatus>(&()).await
    }

    /// Read one split peripheral's status by slot. Requires [`DeviceCapabilities::is_split`];
    /// nothing is sent otherwise.
    pub async fn get_peripheral_status(&self, slot: u8) -> Result<PeripheralStatus, RynkHostError> {
        if !self.capabilities.is_split {
            return Err(RynkHostError::Unsupported(
                Cmd::GetPeripheralStatus,
                "not a split keyboard",
            ));
        }
        self.request::<command::GetPeripheralStatus>(&slot).await
    }

    /// Read the current words-per-minute estimate.
    pub async fn get_wpm(&self) -> Result<u16, RynkHostError> {
        self.request::<command::GetWpm>(&()).await
    }

    /// Read the firmware's sleep state.
    pub async fn get_sleep_state(&self) -> Result<bool, RynkHostError> {
        self.request::<command::GetSleepState>(&()).await
    }

    /// Read the host LED indicator state (caps/num/scroll lock, etc.).
    pub async fn get_led_indicator(&self) -> Result<LedIndicator, RynkHostError> {
        self.request::<command::GetLedIndicator>(&()).await
    }

    /// Read the active connection type (USB / BLE).
    pub async fn get_connection_type(&self) -> Result<ConnectionType, RynkHostError> {
        self.request::<command::GetConnectionType>(&()).await
    }

    /// Read the full connection status. This is the same payload the
    /// `ConnectionChange` topic pushes; use it to catch up after a missed
    /// push.
    pub async fn get_connection_status(&self) -> Result<ConnectionStatus, RynkHostError> {
        self.request::<command::GetConnectionStatus>(&()).await
    }

    /// Read BLE status (active profile, connection state). Requires
    /// [`DeviceCapabilities::ble_enabled`]; nothing is sent otherwise.
    pub async fn get_ble_status(&self) -> Result<BleStatus, RynkHostError> {
        self.require_ble(Cmd::GetBleStatus)?;
        self.request::<command::GetBleStatus>(&()).await
    }

    /// Switch to a BLE profile by slot. Requires [`DeviceCapabilities::ble_enabled`];
    /// nothing is sent otherwise.
    pub async fn switch_ble_profile(&self, slot: u8) -> Result<(), RynkHostError> {
        self.require_ble(Cmd::SwitchBleProfile)?;
        self.request::<command::SwitchBleProfile>(&slot).await
    }

    /// Clear (unbond) a BLE profile by slot. Clearing the currently connected profile
    /// drops that connection. Requires [`DeviceCapabilities::ble_enabled`]; nothing is
    /// sent otherwise.
    pub async fn clear_ble_profile(&self, slot: u8) -> Result<(), RynkHostError> {
        self.require_ble(Cmd::ClearBleProfile)?;
        self.request::<command::ClearBleProfile>(&slot).await
    }
}

#[cfg(feature = "alloc")]
impl Client {
    /// Read the whole keymap — every layer, in [`get_keymap_bulk`](Self::get_keymap_bulk)
    /// order — with concurrent paged reads. A short page ends the read early.
    pub async fn read_all_keymap(&self) -> Result<Vec<KeyAction>, RynkHostError> {
        let caps = self.capabilities;
        let (rows, cols) = (caps.num_rows as u16, caps.num_cols as u16);
        let total = caps.num_layers as usize * rows as usize * cols as usize;
        self.read_all(total, caps.max_bulk_keys.into(), async |c, start| {
            let (layer, row, col) = keymap_pos(start, rows, cols);
            c.get_keymap_bulk(layer, row, col).await.map(|r| r.actions)
        })
        .await
    }

    /// Read every combo slot with concurrent paged reads. A short page ends the read early.
    pub async fn read_all_combos(&self) -> Result<Vec<Combo>, RynkHostError> {
        let total = self.capabilities.max_combos as usize;
        self.read_all(total, self.capabilities.max_bulk_items.into(), async |c, start| {
            c.get_combo_bulk(start as u8).await.map(|r| r.configs)
        })
        .await
    }

    /// Read every morse slot with concurrent paged reads. A short page ends the read early.
    pub async fn read_all_morses(&self) -> Result<Vec<Morse>, RynkHostError> {
        let total = self.capabilities.max_morse as usize;
        self.read_all(total, self.capabilities.max_bulk_items.into(), async |c, start| {
            c.get_morse_bulk(start as u8).await.map(|r| r.configs)
        })
        .await
    }

    /// Write the whole keymap with concurrent paged writes, each page filled up to the
    /// device's payload limit. A failure leaves the earlier pages applied.
    pub async fn write_all_keymap(&self, actions: Vec<KeyAction>) -> Result<(), RynkHostError> {
        let caps = self.capabilities;
        let (rows, cols) = (caps.num_rows as u16, caps.num_cols as u16);
        // 3 fixed bytes before the items: layer, start_row, start_col.
        self.write_all(Cmd::SetKeymapBulk, 3, actions, async |c, start, actions| {
            let (layer, row, col) = keymap_pos(start, rows, cols);
            c.set_keymap_bulk(SetKeymapBulkRequest {
                layer,
                start_row: row,
                start_col: col,
                actions,
            })
            .await
        })
        .await
    }

    /// Write every combo with concurrent paged writes, each page filled up to the
    /// device's payload limit. A failure leaves the earlier pages applied.
    pub async fn write_all_combos(&self, configs: Vec<Combo>) -> Result<(), RynkHostError> {
        // 1 fixed byte before the items: start_index.
        self.write_all(Cmd::SetComboBulk, 1, configs, async |c, start, configs| {
            c.set_combo_bulk(SetComboBulkRequest {
                start_index: start as u8,
                configs,
            })
            .await
        })
        .await
    }

    /// Write every morse with concurrent paged writes, each page filled up to the
    /// device's payload limit. A failure leaves the earlier pages applied.
    pub async fn write_all_morses(&self, configs: Vec<Morse>) -> Result<(), RynkHostError> {
        // 1 fixed byte before the items: start_index.
        self.write_all(Cmd::SetMorseBulk, 1, configs, async |c, start, configs| {
            c.set_morse_bulk(SetMorseBulkRequest {
                start_index: start as u8,
                configs,
            })
            .await
        })
        .await
    }

    /// Read a whole resource on [`MAX_IN_FLIGHT`] lanes concurrently.
    /// `advertised` is the device's max items per page (`max_bulk_items`/`max_bulk_keys`).
    ///
    /// Each lane claims a `spacing`-size "window" and the windows together cover all the data.
    /// The parked size is considered when calculating the window. See [`PARKED_REQUEST_BYTES`].
    async fn read_all<Item>(
        &self,
        total: usize,
        advertised: usize,
        fetch: impl AsyncFn(&Self, u16) -> Result<Vec<Item>, RynkHostError>,
    ) -> Result<Vec<Item>, RynkHostError> {
        let frame = RYNK_HEADER_SIZE + self.capabilities.max_payload_size as usize;
        let spacing = (advertised * frame.saturating_sub(MAX_IN_FLIGHT * PARKED_REQUEST_BYTES) / frame).max(1);
        let next = AtomicUsize::new(0);
        let lanes = join_array(core::array::from_fn::<_, MAX_IN_FLIGHT, _>(|_| async {
            let mut pages = Vec::new();
            loop {
                let start = next.fetch_add(1, Ordering::Relaxed).saturating_mul(spacing);
                if start >= total {
                    break Ok::<_, RynkHostError>(pages);
                }
                // Cover the window: parked requests squeeze the firmware's replies, and
                // walking away from a short page leaves a gap the stitch below would
                // read as end of data.
                let window = (start + spacing).min(total);
                let (mut cursor, mut retries) = (start, 0);
                while cursor < window {
                    let page = match fetch(self, cursor as u16).await {
                        // A reply window squeezed to nothing at all comes back as `Busy`.
                        Err(RynkHostError::Rejected(RynkError::Busy)) if retries < 16 => {
                            retries += 1;
                            continue;
                        }
                        page => page?,
                    };
                    // Only a start past the device's last item pages empty.
                    let len = page.len();
                    if len == 0 {
                        break;
                    }
                    pages.push((cursor, page));
                    cursor += len;
                }
            }
        }))
        .await;
        let mut pages: Vec<(usize, Vec<Item>)> = Vec::new();
        for lane in lanes {
            pages.extend(lane?);
        }
        pages.sort_unstable_by_key(|(start, _)| *start);
        let mut out = Vec::with_capacity(total);
        for (start, page) in pages {
            if start > out.len() {
                break; // an earlier window hit an empty page — the device has no more items
            }
            let skip = out.len() - start;
            out.extend(page.into_iter().skip(skip));
        }
        Ok(out)
    }

    /// Write a whole resource as non-overlapping pages, up to [`MAX_IN_FLIGHT`] in flight.
    /// `fixed` is the request bytes before the item list; a failed lane stops, others go on.
    async fn write_all<Item: Serialize>(
        &self,
        cmd: Cmd,
        fixed: usize,
        items: Vec<Item>,
        store: impl AsyncFn(&Self, u16, Vec<Item>) -> Result<(), RynkHostError>,
    ) -> Result<(), RynkHostError> {
        let mut pages = split_pages(cmd, fixed, self.capabilities.max_payload_size as usize, items)?;
        // One round trip per page however full, so an even static split balances the lanes.
        let chunk = pages.len().div_ceil(MAX_IN_FLIGHT);
        let lanes: [Vec<_>; MAX_IN_FLIGHT] = core::array::from_fn(|_| pages.drain(..chunk.min(pages.len())).collect());
        let store = &store; // the lanes move their own pages in, so `store` is shared by reference
        join_array(lanes.map(|pages| async move {
            for (start, page) in pages {
                store(self, start, page).await?;
            }
            Ok(())
        }))
        .await
        .into_iter()
        .collect()
    }
}

/// Split `items` into write pages tagged with their first item's index. Sizing by real
/// encoded size fits several times more per frame than the advertised count, which must
/// assume worst-case items; an item too big alone returns [`RynkHostError::Encode`].
#[cfg(feature = "alloc")]
fn split_pages<Item: Serialize>(
    cmd: Cmd,
    fixed: usize,
    budget: usize,
    items: Vec<Item>,
) -> Result<Vec<(u16, Vec<Item>)>, RynkHostError> {
    let mut pages = Vec::new();
    let mut page = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (i, item) in items.into_iter().enumerate() {
        let size = serialized_size(&item).map_err(|_| RynkHostError::Encode(cmd))?;
        // A fresh page spends one byte on the count, so this weighs the item alone.
        if fixed + 1 + size > budget {
            return Err(RynkHostError::Encode(cmd));
        }
        // postcard's count varint widens as the page fills; a scalar measures the same.
        let count_bytes = serialized_size(&(page.len() + 1)).map_err(|_| RynkHostError::Encode(cmd))?;
        if fixed + count_bytes + used + size > budget {
            pages.push((start as u16, core::mem::take(&mut page)));
            (start, used) = (i, 0);
        }
        used += size;
        page.push(item);
    }
    if !page.is_empty() {
        pages.push((start as u16, page));
    }
    Ok(pages)
}

/// Convert a flat key index (counting columns, then rows, then layers) into a
/// `(layer, row, col)` address for a `rows`×`cols` keyboard. The index is
/// `u16` because a keymap can have more than 255 keys; each address component
/// still fits in a `u8`.
#[cfg(feature = "alloc")]
fn keymap_pos(cursor: u16, rows: u16, cols: u16) -> (u8, u8, u8) {
    let layer = cursor / (rows * cols);
    let row = (cursor / cols) % rows;
    let col = cursor % cols;
    (layer as u8, row as u8, col as u8)
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    /// Split `items`, then verify every page: pages cover `items` in order
    /// with no gaps, none is empty, and each page's payload fits `budget`.
    /// Returns the page count.
    fn assert_packed<Item: Serialize + Clone>(fixed: usize, budget: usize, items: &[Item]) -> usize {
        let pages = split_pages(Cmd::SetComboBulk, fixed, budget, items.to_vec()).unwrap();
        let mut expected_start = 0;
        for (start, page) in &pages {
            assert_eq!(*start as usize, expected_start);
            assert!(!page.is_empty());
            let bytes: usize = page.iter().map(|i| serialized_size(i).unwrap()).sum();
            let count = serialized_size(&page.len()).unwrap();
            assert!(fixed + count + bytes <= budget, "page at {start} over budget");
            expected_start += page.len();
        }
        assert_eq!(expected_start, items.len());
        pages.len()
    }

    #[test]
    fn packs_by_real_encoded_size() {
        // Each item encodes to 101 bytes (1 length byte + 100 data bytes), so
        // a 482-byte budget fits 4 per page and 10 items take 3 pages. Paging
        // by an advertised count of 2 would take 5.
        let items = vec![vec![0x11u8; 100]; 10];
        assert_eq!(assert_packed(1, 482, &items), 3);
    }

    #[test]
    fn count_varint_growth_is_budgeted() {
        // 1-byte items, so the count varint growing from 1 to 2 bytes at 128
        // items affects what fits.
        let items = vec![0u8; 300];
        assert_packed(1, 132, &items);
    }

    #[test]
    fn one_oversized_item_is_an_encode_error() {
        let items = vec![vec![0u8; 500]];
        assert!(matches!(
            split_pages(Cmd::SetComboBulk, 1, 482, items),
            Err(RynkHostError::Encode(_))
        ));
        // Also when it is not the first item of its page.
        let items = vec![vec![0u8; 100], vec![0u8; 500]];
        assert!(matches!(
            split_pages(Cmd::SetComboBulk, 1, 482, items),
            Err(RynkHostError::Encode(_))
        ));
    }

    #[test]
    fn empty_items_pack_to_no_pages() {
        assert!(split_pages::<u8>(Cmd::SetComboBulk, 1, 482, vec![]).unwrap().is_empty());
    }

    /// The `fixed` argument each `write_all_*` passes is hand-counted from its
    /// request struct, so adding a field there would silently overfill pages.
    /// postcard writes a struct as its fields back to back, so an empty request
    /// measures `fixed` plus the one-byte item count of an empty list.
    #[test]
    fn fixed_prefix_matches_request_layout() {
        let keymap = SetKeymapBulkRequest {
            layer: 0,
            start_row: 0,
            start_col: 0,
            actions: Vec::new(),
        };
        assert_eq!(serialized_size(&keymap).unwrap(), 3 + 1);
        let combo = SetComboBulkRequest {
            start_index: 0,
            configs: Vec::new(),
        };
        assert_eq!(serialized_size(&combo).unwrap(), 1 + 1);
        let morse = SetMorseBulkRequest {
            start_index: 0,
            configs: Vec::new(),
        };
        assert_eq!(serialized_size(&morse).unwrap(), 1 + 1);
    }
}
