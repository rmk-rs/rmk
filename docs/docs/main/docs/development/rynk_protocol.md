<!-- GENERATED — do not edit. Rendered from the `endpoints!`/`topics!` tables in
     rmk-types/src/protocol/rynk/command.rs. Regenerate with:
     UPDATE_SNAPSHOTS=1 cargo test -p rmk-types --features rynk protocol_reference -->

# Rynk Protocol Reference

Current protocol version: **0.1**.

Every transport (USB CDC, BLE GATT, BLE HID) carries the same frame — a 3-byte header plus a [postcard](https://docs.rs/postcard)-encoded payload:

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
