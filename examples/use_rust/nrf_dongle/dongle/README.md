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

The dongle scans for a keyboard whenever it has none to serve: at power-on with
no bond, and after a bonded keyboard fails to answer. A keyboard that pairs in
that window replaces whatever the dongle was bonded to, so swapping keyboards
needs nothing on the host side. Each scan lasts 30s
([`dongle_pairing_window_secs`](../../../../docs/docs/main/docs/configuration/rmk_config.md)),
and ends early if the bonded keyboard turns up after all.
