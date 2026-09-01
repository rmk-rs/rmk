<!-- GENERATED — do not edit. Rendered from the `endpoints!`/`topics!` tables in
     rmk-types/src/protocol/rynk/command.rs by the template in rmk-types/src/protocol/rynk/tests.rs.
     Regenerate from the rmk-types/ directory with:
     UPDATE_SNAPSHOTS=1 cargo test -p rmk-types --features rynk protocol_reference -->

# Rynk Protocol Reference

Current protocol version: **0.1**.

Every transport (USB vendor bulk, BLE GATT, BLE HID) carries the same frame — a 3-byte header plus a [postcard](https://docs.rs/postcard)-encoded payload:

```text
┌──────────────┬───────────┐
│  CMD u16 LE  │  SEQ u8   │  ← 3-byte header
├──────────────┴───────────┤
│ postcard-encoded payload │
└──────────────────────────┘
```

On the wire the whole frame is COBS-encoded and terminated by a single `0x00` delimiter, so the byte stream is self-synchronizing.

- **Requests** use CMD `0x0000..=0x7FFF`. The response echoes CMD and SEQ and wraps its payload in postcard `Result<T, RynkError>` (`T = ()` for `Set*`).
- **Topics** use CMD `0x8000..=0xFFFF` (server → host push, SEQ `0`, bare payload).

Which commands a firmware answers depends on the RMK Cargo features it was built with: a row with no **Feature** is present once `rynk` is on, and the rest need their feature (`_ble`, `split`, …) compiled in. A command the firmware wasn't built with answers `UnknownCmd`.

## Transports

The same COBS-framed byte stream runs over every transport; only how a host finds and opens the link differs.

| Transport       | How the host reaches it                                                                                                                                                                                                                                                                                                                                                            |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| USB vendor bulk | A vendor-specific interface with class/subclass/protocol `0xFF`/`0x52`/`0x52` and one bulk IN + one bulk OUT endpoint. Hosts discover keyboards by that interface triple, not by VID/PID. An MS OS 2.0 descriptor binds it to WinUSB, so Windows needs no driver. The firmware also prefixes its USB serial number with `rynk:` as an informational marker.                        |
| BLE GATT        | Service `10900067-537f-4f0a-9b55-929e271f61ab` with two characteristics: the host writes request bytes to `output_data` (`19802524-6f90-4346-93c2-63dbc509ab55`) and subscribes to notifications on `input_data` (`80f9319b-0c74-43a5-9738-c59d6dda3db9`). Both require an encrypted link. A single write or notification carries at most 244 bytes; a longer frame spans several. |
| BLE HID         | A vendor HID report (usage page `0xFF14`, usage `0x61`) alongside the keyboard's HID-over-GATT service, so a bonded keyboard is reachable through the OS HID stack (for example WebHID) without a second pairing. Each report is exactly 32 bytes: the host splits a frame across reports and zero-pads the last one, and the receiver treats padding as empty COBS frames.        |

A [dongle](../features/dongle) relays these frames untouched, so a host talks to the dongle's USB interface exactly as it would to the keyboard.

## Sizing and bulk transfer

Each peer holds one frame in a buffer of `rynk_buffer_size` bytes (a `[rmk]` option, see [RMK config](../configuration/rmk_config#rynk-protocol-configuration)). The largest payload a frame can carry is what remains after COBS overhead, the delimiter, and the 3-byte header; the firmware reports it as `DeviceCapabilities.max_payload_size`. Read the capabilities and size requests from them rather than assuming a fixed limit.

`DeviceCapabilities` also advertises `bulk_transfer_supported` and the paging strides `max_bulk_keys` (worst-case keys per `GetKeymapBulk` page) and `max_bulk_items` (worst-case entries per `GetComboBulk`/`GetMorseBulk` page). A bulk read names a start — for the keymap `(layer, row, col)`, read forward through the flat row-major, layer-major keymap; for combos and morses a slot index — and returns as many consecutive entries as fit in one payload, or fewer at the end. A host pages by advancing its start by the stride; a short page ends the read. A bulk write carries a start plus a list of entries and is packed by encoded size up to `max_payload_size`. A reply that does not fit beside other pipelined requests answers `Busy`; retry once they complete.

`GetLayout` serves the compressed layout blob 244 bytes per call: the request is a byte offset and `LayoutChunk` carries `total_len` plus that page's bytes. Macros move in `macro_chunk_size` pieces (`protocol_macro_chunk_size` in `[rmk]`) addressed by byte offset.

## Errors

A request's response is postcard `Result<T, RynkError>`; the `Err` side is one of these variants.

| Variant         | Meaning                                                                                                         |
| --------------- | --------------------------------------------------------------------------------------------------------------- |
| `Malformed`     | The request could not be decoded.                                                                               |
| `NotReady`      | The device is not in a state to satisfy the request.                                                            |
| `StorageFault`  | Persistent storage failed on a write (flash erase/write error).                                                 |
| `Internal`      | Internal firmware fault.                                                                                        |
| `Unimplemented` | The command is recognized but its handler is not implemented yet.                                               |
| `Invalid`       | The request decoded cleanly but is semantically invalid (out-of-range index, bad value).                        |
| `UnknownCmd`    | The frame is well-formed but its CMD is unknown to this firmware.                                               |
| `Locked`        | The command is gated by the lock and this session is locked (see Lock).                                         |
| `Busy`          | Transient backpressure: the reply did not fit beside pipelined requests still queued. Retry once they complete. |

## Lock

Commands that can flash firmware, wipe storage, or read the matrix sit behind a physical-presence unlock. `BootloaderJump`, `StorageReset`, `GetMatrixState`, and (with `_ble`) `ClearBleProfile` always need an unlocked session; every `Set*` command joins them when the firmware was built with `[host] write_requires_unlock = true`. A gated command on a locked session answers `Locked` and does nothing. `GetLockStatus`, `UnlockPoll`, and `Lock` are never gated.

The lock is per session and starts locked; `Lock` or the end of the session (unplug, BLE disconnect) relocks it. To unlock, a host polls `UnlockPoll` while the user holds the challenge keys that `LockStatus.key_positions` reports (`[host].unlock_keys`); the session is unlocked once `locked` clears. With no `unlock_keys` configured the challenge is empty and the gated commands can never be unlocked; a firmware built with `[host] insecure = true` starts unlocked and ignores `Lock`. See [Rynk](../features/rynk#locking-dangerous-operations) for the user-facing side.

## Endpoints

| CMD      | Name                  | Request                | Response                | Feature | Notes                                                                        |
| -------- | --------------------- | ---------------------- | ----------------------- | ------- | ---------------------------------------------------------------------------- |
| `0x0001` | `GetVersion`          | `()`                   | `ProtocolVersion`       |         |                                                                              |
| `0x0002` | `GetCapabilities`     | `()`                   | `DeviceCapabilities`    |         |                                                                              |
| `0x0003` | `Reboot`              | `()`                   | `()`                    |         |                                                                              |
| `0x0004` | `BootloaderJump`      | `()`                   | `()`                    |         |                                                                              |
| `0x0005` | `StorageReset`        | `StorageResetMode`     | `()`                    |         |                                                                              |
| `0x0006` | `GetLockStatus`       | `()`                   | `LockStatus`            |         | Pure read of the current lock state — no side effects.                       |
| `0x0007` | `UnlockPoll`          | `()`                   | `LockStatus`            |         | Arms/refreshes the unlock attempt and samples the held challenge keys.       |
| `0x0008` | `Lock`                | `()`                   | `()`                    |         | Relock immediately.                                                          |
| `0x0009` | `GetLayout`           | `u32`                  | `LayoutChunk`           |         | Get layout blob chunk. `u32` is the byte offset.                             |
| `0x000A` | `GetDeviceInfo`       | `()`                   | `DeviceInfo`            |         | Identity strings and USB ids; feature gating stays in `GetCapabilities`.     |
| `0x0101` | `GetKeyAction`        | `KeyPosition`          | `KeyAction`             |         |                                                                              |
| `0x0102` | `SetKeyAction`        | `SetKeyRequest`        | `()`                    |         |                                                                              |
| `0x0103` | `GetDefaultLayer`     | `()`                   | `u8`                    |         |                                                                              |
| `0x0104` | `SetDefaultLayer`     | `u8`                   | `()`                    |         |                                                                              |
| `0x0105` | `GetEncoderAction`    | `GetEncoderRequest`    | `EncoderAction`         |         |                                                                              |
| `0x0106` | `SetEncoderAction`    | `SetEncoderRequest`    | `()`                    |         |                                                                              |
| `0x0107` | `GetKeymapBulk`       | `GetKeymapBulkRequest` | `GetKeymapBulkResponse` |         |                                                                              |
| `0x0108` | `SetKeymapBulk`       | `SetKeymapBulkRequest` | `()`                    |         |                                                                              |
| `0x0201` | `GetMacro`            | `GetMacroRequest`      | `MacroData`             |         |                                                                              |
| `0x0202` | `SetMacro`            | `SetMacroRequest`      | `()`                    |         |                                                                              |
| `0x0301` | `GetCombo`            | `u8`                   | `Combo`                 |         |                                                                              |
| `0x0302` | `SetCombo`            | `SetComboRequest`      | `()`                    |         |                                                                              |
| `0x0303` | `GetComboBulk`        | `GetComboBulkRequest`  | `GetComboBulkResponse`  |         |                                                                              |
| `0x0304` | `SetComboBulk`        | `SetComboBulkRequest`  | `()`                    |         |                                                                              |
| `0x0401` | `GetMorse`            | `u8`                   | `Morse`                 |         |                                                                              |
| `0x0402` | `SetMorse`            | `SetMorseRequest`      | `()`                    |         |                                                                              |
| `0x0403` | `GetMorseBulk`        | `GetMorseBulkRequest`  | `GetMorseBulkResponse`  |         |                                                                              |
| `0x0404` | `SetMorseBulk`        | `SetMorseBulkRequest`  | `()`                    |         |                                                                              |
| `0x0501` | `GetFork`             | `u8`                   | `Fork`                  |         |                                                                              |
| `0x0502` | `SetFork`             | `SetForkRequest`       | `()`                    |         |                                                                              |
| `0x0601` | `GetBehaviorConfig`   | `()`                   | `BehaviorConfig`        |         |                                                                              |
| `0x0602` | `SetBehaviorConfig`   | `BehaviorConfig`       | `()`                    |         |                                                                              |
| `0x0701` | `GetConnectionType`   | `()`                   | `ConnectionType`        |         |                                                                              |
| `0x0702` | `GetConnectionStatus` | `()`                   | `ConnectionStatus`      |         | Full `ConnectionStatus` snapshot.                                            |
| `0x0703` | `GetBleStatus`        | `()`                   | `BleStatus`             | `_ble`  |                                                                              |
| `0x0704` | `SwitchBleProfile`    | `u8`                   | `()`                    | `_ble`  |                                                                              |
| `0x0705` | `ClearBleProfile`     | `u8`                   | `()`                    | `_ble`  |                                                                              |
| `0x0801` | `GetCurrentLayer`     | `()`                   | `u8`                    |         |                                                                              |
| `0x0802` | `GetMatrixState`      | `()`                   | `MatrixState`           |         |                                                                              |
| `0x0803` | `GetBatteryStatus`    | `()`                   | `BatteryStatus`         | `_ble`  |                                                                              |
| `0x0804` | `GetPeripheralStatus` | `u8`                   | `PeripheralStatus`      | `split` |                                                                              |
| `0x0805` | `GetWpm`              | `()`                   | `u16`                   |         | Latest WPM, sourced from the `WpmUpdate` topic snapshot.                     |
| `0x0806` | `GetSleepState`       | `()`                   | `bool`                  |         | Latest sleep flag, sourced from the `SleepState` topic snapshot.             |
| `0x0807` | `GetLedIndicator`     | `()`                   | `LedIndicator`          |         | Latest HID LED bitmap, sourced from the `LedIndicatorChange` topic snapshot. |

## Topics

Topics are best-effort pushes; the `Get*` endpoints above mirror their payloads so a host can recover a missed push.

| CMD      | Name                  | Payload            | Feature | Notes |
| -------- | --------------------- | ------------------ | ------- | ----- |
| `0x8001` | `LayerChange`         | `u8`               |         |       |
| `0x8002` | `WpmUpdate`           | `u16`              |         |       |
| `0x8003` | `ConnectionChange`    | `ConnectionStatus` |         |       |
| `0x8004` | `SleepState`          | `bool`             |         |       |
| `0x8005` | `LedIndicatorChange`  | `LedIndicator`     |         |       |
| `0x8006` | `BatteryStatusChange` | `BatteryStatus`    | `_ble`  |       |
