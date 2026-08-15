# RMK tri-mode dongle on nRF54LM20A

A BLE central that relays its bonded RMK keyboard to the host over USB: HID
reports, plus Rynk config frames passed through byte-for-byte.

Any `rynk` BLE keyboard of a matching RMK version can pair with it; see
[`../central`](../central) for the split-keyboard central in this example.

## Flash

```shell
cargo build --release
# via J-Link:
./run.sh target/thumbv8m.main-none-eabihf/release/rmk-nrf54lm20-dongle
```

A new keyboard pairs only while a window is open: once for 30s at power-on
([`dongle_pairing_window_secs`](../../../../docs/docs/main/docs/configuration/rmk_config.md)),
and continuously while the dongle has no bond. 
The power-on window ends early if the bonded keyboard turns up after all. 
Outside those windows the dongle only reconnects its bonded keyboard. 
To pair with a new dongle: hold the keyboard's dongle key for 5s, then replug the dongle. 
