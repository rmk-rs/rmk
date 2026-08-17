# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Connectivity

- Add BLE dongle support (`dongle` Cargo feature). A dongle is a USB-attached nRF board that relays a wireless keyboard's HID reports to the host, so the keyboard works on machines with no usable Bluetooth. The keyboard reserves a bond slot of its own for the dongle — profile cycling never reaches it — and `User(N+5)` (`SwitchToDongle`) switches to it; holding that key for 5 seconds clears the bond and goes looking for another dongle. The dongle also relays the host protocol, so Vial and Rynk keep working through it ([#1028](https://github.com/rmk-rs/rmk/pull/1028), [#1047](https://github.com/rmk-rs/rmk/pull/1047)). See [examples/use_rust/nrf_dongle](https://github.com/rmk-rs/rmk/tree/main/examples/use_rust/nrf_dongle)
- Add nRF54 support (nRF54L15 and nRF54LM20)
- Add ESP32-H2 support
- Add an `nrf52820_ble` chip feature ([#1049](https://github.com/rmk-rs/rmk/pull/1049))
- Add SiFli SF32LB52x BLE support
- Hold a BLE profile key for 5 seconds to forget that profile's bond and re-pair, so you can move a profile to a different host without wiping storage ([#1045](https://github.com/rmk-rs/rmk/pull/1045))
- Add BLE passkey handling on the initial connection, for hosts that require pairing confirmation ([#756](https://github.com/rmk-rs/rmk/issues/756))
- Add an option to disable the BLE battery service
- Report the split peripheral's battery level to the central, and configure the peripheral's battery ADC from `keyboard.toml`
- Advertise the boot keyboard protocol on the primary HID interface, so the keyboard works in BIOS/UEFI and other boot-protocol-only hosts
- Add USB logging support for split peripherals ([#1000](https://github.com/rmk-rs/rmk/pull/1000))

#### Displays and other outputs

- Add OLED display support (`display` Cargo feature) with a `[display]` section in `keyboard.toml`. Ships `oled_async` drivers for SH1106, SH1107, SH1108 and SSD1309, plus `lcd-async` for color panels. Default renderers show layer, connection, and battery state, blank while the keyboard sleeps, and honour configurable render/poll intervals. Custom renderers are supported — see [examples/use_rust/custom_renderer](https://github.com/rmk-rs/rmk/tree/main/examples/use_rust/custom_renderer). On splits, the central forwards display state events to peripherals so both halves can show the same status
- Add Plover HID stenography support (`steno` Cargo feature): a steno HID descriptor and USB endpoint, an `Action::Steno` keycode with `STN(key)` syntax in `keyboard.toml`, and a live-state reporter

#### Behaviors

- Add `bootmagic` config: hold a designated key during boot to drop into the chip bootloader. Works on unibody and on each half of a split independently. Particularly useful for split peripherals whose BOOTSEL button is physically inaccessible ([#457](https://github.com/rmk-rs/rmk/issues/457)).
- Make `rmk::boot` module public so user code can call `boot::jump_to_bootloader()` directly
- Add auto mouse layer behavior: automatically activate a configured layer when X/Y cursor motion from a pointing device is detected, and deactivate it after a `timeout` of inactivity ([#781](https://github.com/rmk-rs/rmk/issues/781)) with `deactivate_on_key` (with `extra_mouse_keys`) and `reset_timeout_on_key` options; entry capacity is auto-derived from `keyboard.toml`, overridable via `[rmk].auto_mouse_layer_max_num`
- Add `quick_tap_timeout` morse profile option: re-pressing a morse/tap-hold key within the window after its last release fires the tap action immediately and keeps it held, so the OS auto-repeats the tap instead of triggering the hold action
- Add one-shot sticky modifiers, with a `quick_release` option under `[behavior.one_shot_modifiers]`
- Add a `prior_idle_time` cooldown for combos: a combo won't fire if another key was pressed within the window, which keeps fast rolls from triggering combos accidentally
- Support nested actions in tap-hold / morse configuration
- Add `PDF(n)` (set default layer) to `keyboard.toml` keymaps
- Add a bilateral marker to `[layout].map`, so same-hand modifiers still work when `unilateral_tap` is enabled
- Allow overriding `flow_tap` per morse profile
- Expose held modifier combinations in config ([#999](https://github.com/rmk-rs/rmk/pull/999))
- Add conversion for space cadet keys
- Support the full extended Vial macro format ([#989](https://github.com/rmk-rs/rmk/pull/989))
- Add `[host].insecure` (renamed from `vial_insecure`, which still parses) to start the host configurator unlocked
- Embed a version stamp in the USB serial number, so a host can tell which firmware build it is talking to
- Shrink the in-RAM keymap by storing each tap-hold's timing profile as a `u8` index into a small deduplicated morse profile table (`MorsesConfig::profiles`) instead of an inline 8-byte `MorseProfile`. `KeyAction::TapHold` drops from 16 to 7 bytes, which also removes the profile's 8-byte alignment padding across every `KeyAction`-sized buffer — roughly a 3 KB RAM saving on a 5×14×5 board at no flash cost. An index with no table entry falls back to the default profile. Table capacity is configurable via `[rmk] morse_profile_max_num` (default 16, max 255); `keyboard.toml` users keep referencing profiles by name (the macro interns them automatically)

#### Input devices

- Add Azoteq IQS5xx (IQS550 / IQS572 / IQS525) trackpad driver, used by Azoteq's TPS43/TPS65 modules. Supports operation with or without an `RDY` pin and is configurable via `keyboard.toml` on nRF52 / RP2040; currently publishes single-finger relative cursor movement only ([#29](https://github.com/rmk-rs/rmk/issues/29))
- Add PMW3360 / PMW3389 optical mouse sensor support
- Add `report_hz` option for Pmw3610Device
- Add per-layer pointing device modes (Cursor / Scroll / Sniper / Caret), so the same trackball scrolls on one layer and moves the cursor on another
- Add opt-in debounce support for rotary encoders

#### Experimental

- Add [Rynk](https://rmk.rs/docs/features/rynk), RMK's native host protocol for on-the-fly configuration over USB and BLE — an alternative to Vial that covers every RMK feature (keymap, layers, encoders, combos, forks, tap-dance/morse, macros, and behavior config), plus live status (current layer, matrix tester, WPM, HID indicators, battery, connection/BLE profile) and device management (reboot, bootloader, storage reset). Enable with the `rynk` Cargo feature and `[host] rynk_enabled = true`; it is mutually exclusive with Vial. Dangerous operations (bootloader, storage reset, matrix tester, clearing a BLE bond) are gated behind a physical-presence unlock (`[host].unlock_keys`). Host client crates live in the new `rynk/` workspace ([#962](https://github.com/rmk-rs/rmk/pull/962)). **Experimental**: the wire protocol and host crates can change in any release, the host crates are not published to crates.io, and there is no ready-made desktop app yet. Vial stays the default
- Rynk `GetBehaviorConfig`/`SetBehaviorConfig` now carry the default morse profile and the flow-tap prior-idle window, so a Rynk host can read and edit the same behavior settings Vial exposes; flow-tap now resolves per-key profile → default profile → `[behavior.morse] enable_flow_tap` ([#1054](https://github.com/rmk-rs/rmk/pull/1054))
- Add DFU firmware update over USB, so a board with a compatible bootloader can be reflashed without pressing BOOTSEL. `dfu_rp` (RP2040) and `dfu_nrf` (nRF52840) pair RMK with the [rmk-boot](https://github.com/rmk-rs/rmk-boot) embassy-boot bootloader, which splits flash into ACTIVE and DFU slots with automatic rollback; `dfu_split` forwards firmware to peripherals over a wired split link; `dfu_lock` gates downloads behind a physical key press. Partition offsets come from the linker symbols in the `rmk-memory.x` that rmk-boot generates (rename it to `memory.x`) — `init_flash_from_linkerscript` reads them at runtime, so no address is ever hardcoded ([#1018](https://github.com/rmk-rs/rmk/pull/1018)). **Experimental**: enabling it repartitions flash and the layout can change in any release. `dfu_split` is wired-split only; combining it with a BLE build is rejected at compile time
- Add `zsa_voyager_bl` for the ZSA Voyager's ignition DFU bootloader

#### Development

- Add a TOML scenario test framework: keyboard behavior cases are written as TOML under `rmk/tests/scenarios/` and expanded into ordinary Rust tests by `run_tests!`, including rotary encoder input. See [the scenario README](https://github.com/rmk-rs/rmk/blob/main/rmk/tests/scenarios/README.md)

### Changed

- **BREAKING**: `run_rmk` is replaced by explicit transports plus `run_all!`. Every component — matrix, storage, input devices, processors, the keyboard, transports, the watchdog — is a runnable handed to a single `run_all!` invocation, instead of being wired through `join`/`run_devices!`/`EVENT_CHANNEL`. Build a `HostService` once, then attach it to a `UsbTransport` and/or `BleTransport` with `.with_host_service(&host_service)`
- **BREAKING**: input devices, input processors, and controllers are unified into one event/processor model. The `Controller`, `EventController`, and `InputProcessor` traits, `run_processor_chain!`, and the central `Event` enum are gone; define events with `#[event]` / `#[derive(Event)]`, input devices with `#[input_device(publish = ...)]`, and handlers with `#[processor(subscribe = [...])]`. `Runnable` moves from `rmk::input_device` to `rmk::core_traits`, and its `run` returns `!`
- **BREAKING**: the BLE transport now owns the BLE stack. `build_ble_stack` and `HostResources` are gone from user code — pass the controller and address to `BleTransport::new(controller, address, rmk_config)` and to `run_rmk_split_peripheral(id, controller, address)`, which no longer takes storage. The ChaCha RNG dependencies (`rand_core` / `rand_chacha`) are no longer needed
- **BREAKING**: the `controller` and `col2row` Cargo features are removed, and `vial_lock` is renamed to `host_lock`
- **BREAKING**: `watchdog` is now a default Cargo feature. It is supported on RP2040, nRF52, and ESP32 and is a no-op elsewhere; `keyboard.toml` users get it automatically, Rust API users pass a watchdog runner to `run_all!`. Disable it by listing your features without `watchdog` under `default-features = false`
- **BREAKING**: the global channel knobs in `[rmk]` (`event_channel_size`, `controller_channel_size`, `controller_channel_pubs`, `controller_channel_subs`) are replaced by per-event settings under `[event.<name>]` (`channel_size`, `pubs`, `subs`)
- **BREAKING**: `keyboard.toml` layout is restructured to separate the electrical matrix, physical layout, and logical keymap. `[layout].matrix_map` becomes `[layout].map`; per-layer key actions move out of `[layout]` (the old `[[layer]]` tables, `keymap`, and `encoder_map`) into a new `[keymap]` table — `[keymap].layers` plus one `[[keymap.layer]]` per layer with `keys` and optional `encoders`. `[layout].rows` and `[layout].cols` are unchanged. Unknown `keyboard.toml` keys are now rejected instead of silently ignored. See the [v0.8 → v0.9 migration guide](https://rmk.rs/docs/migration/v08_v09)
- **BREAKING**: `MorseProfile` (rmk-types) is now packed into a `u64` instead of a `u32` to make room for `quick_tap_timeout`
- **BREAKING**: `behavior.morse.hold_timeout`/`gap_timeout`/`quick_tap_timeout` values above the 13-bit maximum of 8191ms now fail the build with an explicit error
- **BREAKING**: `CompositeReportType` discriminants are renumbered (`Keyboard=1`, `Mouse=2`, `Media=3`, `System=4`): the BLE report map carries the keyboard report as id 1, and the mouse/media/system report ids shift to 2/3/4 on both BLE and USB. USB hosts re-read report ids on every enumeration so nothing changes for them; BLE hosts bonded to an older firmware must forget and re-pair the keyboard
- **BREAKING**: `KeyAction::TapHold` now carries a `u8` profile-table index instead of an inline `MorseProfile`. Pure-Rust keymaps using the custom-profile macros (`thp!`/`mtp!`/`ltp!`/`ttp!`) must populate `behavior.morse.profiles` and pass the 0-based index of the wanted entry; an index with no entry falls back to the default profile. The default-profile macros (`th!`/`mt!`/`lt!`/`tt!`) are unchanged. `keyboard.toml` configs need no changes
- **BREAKING**: `PollingController::INTERVAL` constant is now `PollingProcessor::interval()` method, allowing dynamic interval configuration at runtime
- **BREAKING**: PointingDevice and PointingProcessor replace Pmw3610Device and Pmw3610Processor. For the Pmw3610 the calls of ::new() for these stay the same, only the name changes. If using Rust to configure the keyboard change the calls, if using Toml nothing needs to be done.
- **BREAKING**: `MouseKeyConfig` fields renamed: `time_to_max` → `ticks_to_max`, `wheel_time_to_max` → `wheel_ticks_to_max`, `wheel_max_speed_multiplier` → `wheel_max_speed`
- Update the dependency baseline: `embassy-executor` 0.10 (`platform-cortex-m`), `embassy-nrf` 0.11, `embassy-rp` 0.10, `embassy-stm32` 0.6, `bt-hci` 0.9, `cyw43` 0.7 / `cyw43-pio` 0.10, `esp-hal` 1.1 / `esp-radio` 0.18 / `esp-rtos` 0.3, and a newer `nrf-sdc`. The migration guide lists the matching code changes
- Rynk moves from a CDC ACM interface to its own raw USB vendor interface, so it no longer occupies a serial port and host tools find it by interface class ([#1023](https://github.com/rmk-rs/rmk/pull/1023))
- Refactor mouse key state machine into a dedicated module with per-direction press counts, independent movement/wheel repeat scheduling, and configurable acceleration curves
- Optimize the timing for motion read and sending reports on the PMW3610
- Correct the delay length of PMW3610 to the precise value
- Update `sequential-storage` to v8.0. The on-flash format is unchanged, so existing storage is read back as-is

### Fixed

- Fix mouse keys (`KC_MS_*`), media keys and system control keys doing nothing on Android over BLE: Android's HID host only attaches to the first HID service instance (AOSP `bta_hh_le.cc`, b/286413526), so reports served from the separate composite HID service were never subscribed. All HID reports now live in a single HID service, distinguished by report id via the Report Reference descriptor
- Fix `ClearEeprom` keycode being defined but not functional: pressing the key now resets the storage on release (same operation as `ViaCommand::EepromReset`, requires the `storage` feature), and the keycode round-trips through Vial as `0x7C03` (QMK's `QK_CLEAR_EEPROM`) instead of being rendered as a raw hex literal ([#929](https://github.com/rmk-rs/rmk/issues/929))
- Fix Vial keycode conversion truncating user keycodes to 4 bits: `User16`–`User31` silently aliased to `User0`–`User15` on the Vial side (assign, view, and save-back). Widen the mask and accepted range to 5 bits (`0x7E00..=0x7E1F`, matching QMK's `QK_KB_0..QK_KB_31`) ([#918](https://github.com/rmk-rs/rmk/issues/918))
- Fix BLE output stopping while the keyboard is on charge-only USB power (charge-only cable, wall charger, power bank). A never-enumerated device's bus-idle suspend was published as `Suspended`, which `usb_ready()` treats as routable, so reports were routed to USB endpoints that were never configured and silently dropped ([#910](https://github.com/rmk-rs/rmk/issues/910))
- Fix stuck key when a combo key is re-pressed while the combo is still held. Previously the re-press overwrote the combo output's HID slot, and on combo release the output couldn't be unregistered, leaving the re-pressed key stuck on the host.
- Fix stuck combo output when overlapping triggered combos share a key (e.g. `M+,` and `,+.` both containing Comma). Releasing the shared key now dispatches the release of every fully-unwound combo output, not just the first.
- Fix `unregister_keycode` choosing the wrong HID slot when a combo output and another pressed key share a position. Slot lookup now prefers a `(pos, keycode)` match and falls back to keycode-only.
- Fix spurious "Timer buffer full" warns after 16 distinct key positions are pressed. The per-position timer `LinearMap` is gone; press time is now threaded as a parameter through the morse-press dispatch.
- Fix override attributes (`#[Override(...)]` / `#[Overwritten(...)]`) silently falling back to the generated default: an unknown or miscased variant (e.g. `Entry` instead of `entry`) is now a compile error carrying darling's diagnostic, and an extra attribute on the function (doc comments included) no longer disables a valid override ([#966](https://github.com/rmk-rs/rmk/issues/966))
- Fix `#[Override(bind_interrupt)]` (the form the stm32h7 example documents) never taking effect: the custom interrupt binding is now selected through the shared override matcher instead of being silently dropped in favor of the generated default. The legacy bare `#[bind_interrupt]` marker keeps working unchanged
- Fix non-`defmt` DFU builds (e.g. `dfu_rp` with `usb_log`) failing with a borrow-of-moved-value error on `central_attrs`: embassy-usb's `DfuAttributes` is `Copy` only under `defmt`, so the DFU descriptor now reads the attribute bits before handing the attributes to `DfuState` ([#973](https://github.com/rmk-rs/rmk/issues/973))
- Fix a dropped keypress when two keys resolve to the same HID keycode with different modifiers (a plain bracket and its `SHIFTED()` neighbor). Rolling them quickly sent the same usage in two report slots, which some hosts read as a release of the held key and dropped the second key. Keyboard reports now deduplicate keycodes so the shared usage stays down until the last holder releases it
- Fix the whole input pipeline freezing while the USB host is asleep. On nRF an IN-endpoint write during suspend pends until the host resumes, which parked the report writer and backed up every bounded queue upstream — keyboard task, key event channel, split peripheral manager, GATT notifications — then flushed the backlog at once on resume. RMK now signals remote wakeup and drops the report instead, and keeps retrying the wakeup rather than parking in `wait_resume()` ([#1035](https://github.com/rmk-rs/rmk/pull/1035))
- Fix a one-shot modifier applying to one key too many: a key rolled over before the first shifted key was released still saw the one-shot as active. The modifier is now consumed on the next key press instead of its release, and a held `OSL` consumes a pending one-shot modifier at its own press ([#1036](https://github.com/rmk-rs/rmk/pull/1036))
- Fix a panic in the split central's scan handler when an advertising report carried exactly 25 bytes: the peripheral id is read from byte 25, so the length check let an out-of-bounds index through. Also shorten the central's link supervision timeout from 10s to 5s so a dead peripheral link is detected and reconnected sooner ([#1034](https://github.com/rmk-rs/rmk/pull/1034))
- Drop a BLE connection immediately when the bond info doesn't match the profile, instead of letting the wrong host hold the link ([#1024](https://github.com/rmk-rs/rmk/pull/1024))
- Fix a bonded dongle hijacking the keyboard's BLE host profiles: it connected on the keyboard's bare address, so it also answered the plain HID advertising of a host profile and bonded itself into that slot. The dongle now connects only when the keyboard asks for a dongle, and the keyboard refuses the bonded dongle on every host profile ([#1056](https://github.com/rmk-rs/rmk/pull/1056))
- Fix the nRF SoftDevice Controller memory pool size calculation ([#1002](https://github.com/rmk-rs/rmk/pull/1002))
- Fix spurious sleep, and fix the split peripheral's connection parameters

## [0.8.2] - 2025-12-18

### Added

- Add PMW3610 optical mouse sensor support for nRF and RP2040 with bit-bang SPI
- Add support for configuring static output pins
- DCDC config for nRF52840/nRF52833
- Add `encoder_map` support in `keyboard.toml`
- Add devcontainer config
- Add sitemap to rmk.rs

### Changed

- Make `embedded-hal-async` a required dependency
- Update default BLE connection parameters
- Increase the default number of controller channel pub
- Documentation update

### Fixed

- Fix compilation error when use `Macro()` in keymap config
- Fix row2col matrix doesn't work issue
- Fix `lm` key is not properly released

## [0.8.1] - 2025-11-25

### Changed

- Remove unused `EnumIter` in `rmk-types`

### Fixed

- Fix storage ser/de format error
- Fix a bug of Caps Word

## [0.8.0] - 2025-11-25

### Added

- Add dongle support back, checkout [this example](https://github.com/rmk-rs/rmk/tree/main/examples/use_rust/nrf52840_ble_split_dongle)
- Add `detent` and `pulse` settings to encoder config
- Add `Controller` support for peripheral #584
- Add Fn1(Fn3) + Fn2(Fn3) tri-layer support in Vial
- Add LED indicator and layer state sync from central to peripheral
- Add `vial` and `host` feature
- Add configuration of controller execution mode
- Add capsword support
- Add `default_tx_power` and `use_2m_phy` config for BLE
- Add lock and matrix tester support for Vial
- Add `[host]` config section
- Support changing permissive hold option at the runtime
- Add `detent` and `pulse` config for encoders

### Changed

- Bump lots of dependencies to latest version
- Refactor tap-hold, and introduced morse_actions to tap-dance to support real morse code like tap/hold patterns
- Positional and per key morse profile configuration introduced for tap hold like, morse like keys
- Rename chordal tap to unilateral tap
- Rewrite led indicator, use controller system
- Rename `RapidDebouncer` to `FastDebouncer`
- Remove `col2row`, `bidirectional` and `rapid_debouncer` features
- Use postcard for serialization/deserialization of storage data
- Change central sleep timeout to be in seconds
- Migrate documentation site to rspress

### Fixed

- Fix invalid macro key
- Fix wrong peripheral number setting in Rust split examples
- Fix modifier activation in lm
- Fix combo reorder issue
- Fix key stuck when one shot key rolling with tap hold
- Fix flow-tap misorder
- Fix peripheral message loss

## [0.7.8] - 2025-07-23

### Added

- Hold-on-other-key-press mode for tap-hold
- Add missing keycode to docs

### Changed

- Change `OneShot` as a variant of `Action`

### Fixed

- Permissive hold key rolling error
- Chordal tap triggers tap unexpectly

## [0.7.7] - 2025-07-21

### Added

- [TapDance](https://rmk.rs/docs/features/configuration/behavior.html#tap-dance) support
- Extra delay when executing macros

### Changed

- CI bloat workflow can comment on PR now
- Report battery percentage instead of adc value, and do the report instantly after boot
- Report battery level via BLE only when there's a key action recently

### Fixed

- USB remote wakeup failure
- Overflow in `PollingController`

## [0.7.6] - 2025-07-15

### Changed

- Move encoder events processing to `Keyboard`
- Use bitfield_struct's native defmt formatting
- Use device id as the serial number for nRF
- Move `KeyAction::WithModifier` to `Action::KeyWithModifier`
- ​​Reset the sidebar style in user documentation​

### Added

- Add ESP32 heterogeneous example, which uses ESP32C6 as central and ESP32C3 as peripheral
- Add mouse acceleration support
- Add consts for single-bit structs

### Fixed

- Repeat message from periphral for serial split
- Crash when the host returns empty data
- Key trigger issue when combo is used with one-shot key
- Key trigger issue when there's overlapped combo
- Don't send battery notification according to control point value from host
- Update addr stored in peripheral after re-pairing
- Repeat mouse key when multiple mouse keys are pressed

## [0.7.5] - 2025-07-06

### Fixed

- "No" key with tailing whitespace cannot be parsed
- Key processing error when using tap-hold keys in combo

## [0.7.4] - 2025-07-03

### Added

- Add sleep mode for split central after connected to the host

### Changed

- Refactor key processing, fix tap-hold issues
- Only the valid macro data is stored in the storage now. **Clearing storage is required to update**

### Fixed

- Light service is wrongly disabled
- Correctly update connection parameters after connected to the host
- Remove need for quotes on OSM

## [0.7.3] - 2025-06-18

### Added

- [Logging via USB](https://rmk.rs/docs/features/usb_logging.html)
- Events for controllers

### Changed

- Update to TrouBLE v0.2.0

### Fixed

- Fix sdc build error
- Fix cloud build script for ESP32

## [0.7.2] - 2025-06-12

### Fixed

- Fix ADC initialization for splits
- Fix NonusHash parsing error
- Fix wrong state after switching output
- Fix py32 example

### Added

- Use 2M Phy by default

### Changed

- Move `Controller` behind a feature flag

## [0.7.1] - 2025-06-04

### Fixed

- Fix the error when using matrix_map

## [0.7.0] - 2025-06-04

### Changed

- **BREAKING**: The BLE stack is migrated to [TrouBLE](https://github.com/embassy-rs/trouble/)
- **BREAKING**: Add `rmk-config` and use `[env]` in `.cargo/config.toml` to configure the path of `keyboard.toml`
- Optimize the size of buffer used in USB
- A new documentation site is released! Check out [rmk.rs](https://rmk.rs)

### Added

- BLE and wireless split support for Pi Pico W, check out [this example](https://github.com/rmk-rs/rmk/tree/main/examples/use_config/pi_pico_w_ble)
- Introduce matrix_map for [nicer keyboard matrix configs](https://rmk.rs/docs/features/configuration/layout.html)
- BLE + USB dual-mode support for esp32s3
- Automatically pair between central and peripheral
- Make constants in RMK [configurable via `keyboard.toml`](https://rmk.rs/docs/features/configuration/rmk_config.html)
- Enable [support for keyboard macros](https://rmk.rs/docs/features/keymap/keyboard_macros.html) (via rust based configuration only for now) (closes issues #308, #284, #303, #313, #170)
- Battery charging state reader
- Sleep timeout when advertising
- Allow disabling the storage feature in `Cargo.toml` to work with `keyboard.toml`

### Fixed

- Wrong connection state between splits
- Issue about first adc polling
- Wrong battery status
- Capslock stuck on macOS
- Wrong BLE address setting in `keyboard.toml`

## [0.6.1] - 2025-04-11

### Added

- Repeat key support
- Basic GraveEscape support
- Internal pull-up config for encoders

### Fixed

- Wrong GPIO pulls for stm32
- Combo cannot be triggered correctly when there's overlap between combos
- Battery level led indicator failure

## [0.6.0] - 2025-04-06

### Added

- Input device support
- Rotary encoder and joystick are supported
- State fork behavior
- Bootloader jumping for nRF52 and RP2040
- Artificial pull up resistor to pio tx line
- Shifted key and transparent key support in toml config
- Clear the storage by checking build hash after flashing a new firmware
- stm32g4 example without storage feature

### Changed

- Make `storage` a feature, enabled by default
- Documentation improvement
- Remove unnecessary pio-proc dependency
- Improve modifier reporting

### Fixed

- Wrong ESP32 BLE serial number
- Wrong col/row when using direct pin
- Error when there's empty IO pin list in the config
- Many other minor fixes

## [0.5.2] - 2025-01-22

### Added

- `defmt` feature gate
- rp2350 example
- Added `_matrix` functions to allow passing custom matrix implementation

### Changed

- Make more modules public
- Update embassy dependencies to latest
- Improve robustness of serial communication between splits

### Fixed

- Record positions of triggered keys, fix key stuck
- Remove invalid PHY type setting between splits
- Receive keys from peripheral when there's no connection
- Always sync the connection state to fix the unexpected lost of peripherals
- Fix link scripts which are broken after flip-link updated
- Remove `block_on` to prevent unexpected hang on the periphrals

## [0.5.1] - 2025-01-02

### Added

- Add new [cloud-based project template](https://github.com/rmk-rs/rmk-project-template)
- Connection state sync between central and peripheral
- Use 2M PHY by default
- `mt!` for modifier tap-hold and `th!` for tap-hold action

### Changed

- Use [`rmkit`](https://github.com/rmk-rs/rmkit) as the default project generator
- Update `sequential-storage` to v4.0.0
- Improve CI speed
- Use `cargo-hex-to-uf2` instead of python script for uf2 firmware generation

### Fixed

- Fix hold-after-tap key loss by allowing multi hold-after-tap keys
- Exchange left and right modifier
- Fix wrong number of waited futs of direct-pin matrix
- Fix wrong peripheral conn param by setting conn param from central, not peripheral

## [0.5.0] - 2024-12-16

### Added

- BREAKING: Support `direct_pin` type matrix for split configurations, split pin config is moved to [split.central/peripheral.matrix]
- Support home row mod(HRM) mode with improved tap hold processing
- Add `clear_storage` option
- Enable USB remote wakeup
- py32f07x use_rust example. py32f07x is a super cheap($0.2) cortex-m0 chip from Puya

### Changed

- Remove `rmk-config`

### Fixed

- Fix slightly lag on peripheral side
- Fix invalid BLE state after reconnection on Windows
- Fix ghosting key on macOS
- Fix direct pin debouncer size error
- Fix esp32 input pin pull configuration
- Fix BLE peripheral lag

## [0.4.4] - 2024-11-27

### Fixed

- Fix link error on Windows

## [0.4.3] - 2024-11-25

### Added

- One-shot layer/modifier support
- Tri-layer support

### Fixed

- Fix connection error when there're multiple peripherals
- Fix keycode converter error

## [0.4.2] - 2024-11-13

### Added

- Layout macro to! and df!
- One-shot layer and one-shot modifier
- Make nRF52840 voltage divider configurable
- ch32v307 example

### Changed

- Use `User11` to manually switch between USB mode and BLE mode

### Fixed

- Fix nRF52840 linker scripts for nice!nano
- Fix broken documentation links

## [0.4.1] - 2024-10-31

### Fixed

- Fix lagging for split peripheral

### Added

- Direct pin mod. Including `DirectPinMatrix`, `run_rmk_direct_pin` functions etc.
- Added pin active level parameter `low_active` to direct pin.
- Support no_pin for `DirectPinMatrix`.

## [0.4.0] - 2024-10-28

### Added

- Restart function of ESP32
- Methods for optimizing nRF BLE power consumption, now the idle current is decreased to about 20uA
- Multi-device support for nRF BLE
- New `wm` "With Modifier" macro to support basic keycodes with modifiers active
- Voltage divider to estimate battery voltage
- Per chip/board default settings
- i18n support of documentation
- Use flip-link as default linker

### Changed

- BREAKING: use reference of keymap in `run_rmk`
- BREAKING: refactor the whole macro crate, update `keyboard.toml` config, old `keyboard.toml` config may raise compilation error
- Decouple the matrix(input device) and keyboard implementation
- Stop scanning matrix after releasing all keys
- Move creation of Debouncer and Matrix to `run_rmk_*` function from `initialize_*_and_run`

### Fixed

- Unexpected power consumption for nRF
- Extra memory usage by duplicating keymaps
- A COL/ROW typo
- Stackoverflow of some ESP32 chips by increasing default ESP main stack size

## [0.3.2] - 2024-10-05

### Fixed

- Fix vial not work for nRF

## [0.3.1] - 2024-10-03

### Added

- Automate uf2 firmware generation via `cargo-make`
- Storage and vial support for ESP series
- Vial over BLE support for Windows
- `TO` and `DF` action support

### Changed

- Update `bitfield-struct` to v0.9
- Update `esp32-nimble` to v0.8, as well as used `ESP_IDF_VERSION` to v5.2.3
- Use 0x60000 as the default start addr for nRF52

### Fixed

- Fix no device detected on vial desktop

## [0.3.0] - 2024-09-11

### Changed

- BREAKING: all public keyboard APIs are merged into `run_rmk` and `run_rmk_with_async_flash`. Compared with many different APIs for different chips before, the new API is more self-consistent. Different arguments are enabled by feature gates.

### Added

- Basic split keyboard support via serial and BLE
- ESP32C6 support
- Reboot mechanism for cortex-m chips

## [0.2.4] - 2024-08-06

### Added

- `MatrixTrait` which is used in keyboard instead of a plain `Matrix` struct

### Changed

- Update versions of dependecies

## [0.2.3] - 2024-07-25

### Fixed

- Fix keymap doesn't change issue
- Fix with_modifier action doesn't trigger the key with modifier
- Fix capital letter is not send in keyboard macro

### Changed

- Yield everytime after sending a keyboard report to channel
- Update `sequential-storage` to v3.0.0
- Update `usbd-hid` to v0.7.1

## [0.2.2] - 2024-07-12

- Add keyboard macro support
- Support vial keymap reset command
- Fix default `lt!` and `lm!` implementation

## [0.2.1] - 2024-06-14

### Fixed

- Fix USB not responding when the light service is not enabled

## [0.2.0] - 2024-06-14

### Added

- Support led status update from ble
- Support more nRF chips: nRF52833, nRF52810, nRF52811

## [0.1.21] - 2024-06-08

### Added

- Add `async_matrix` feature, which enables async detection of key press and reduces power consumption

## [0.1.20] - 2024-06-06

### Added

- Support read default keymap from `keyboard.toml`, see https://haobogu.github.io/rmk/keyboard_configuration.html#keymap-config

## [0.1.17] - 2024-06-04

### Fixed

- Fixed doc display error on docs.rs

## [0.1.16] - 2024-06-01

### Added

- Add new CHANGELOG.md
