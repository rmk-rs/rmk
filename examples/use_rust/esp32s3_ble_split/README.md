# esp32s3 BLE SPLIT example

<details>
    <summary>Building and running with rust (same example in esp32s3_ble)</summary>
    <br>
    To run this example, you should have latest Rust in **esp** channel installed. The full instruction of installing esp Rust toolchain can be found [here](https://docs.esp-rs.org/book/installation/index.html).

    [`espflash`](https://github.com/esp-rs/espflash) should also be installed:

    ```
    cargo install cargo-espflash espflash
    ```

    After having everything installed, use the following command to run the example:

    ```
    cd examples/use_rust/esp32s3_ble_split
    cargo +esp run --release
    ```

    If everything is good, you'll see the log as the following:

    ```shell
    cargo run --release
        Compiling ...
        ...
        ...
        Finished `release` profile [optimized + debuginfo] target(s) in 11.70s
        Running `espflash flash --monitor --port /dev/cu.usbmodem211401 target/xtensa-esp32s3-none-elf/release/rmk-esp32s3`
    [2025-04-10T10:01:23Z INFO ] Serial port: '/dev/cu.usbmodem211401'
    [2025-04-10T10:01:23Z INFO ] Connecting...
    [2025-04-10T10:01:23Z INFO ] Using flash stub
    Chip type:         esp32s3 (revision v0.1)
    Crystal frequency: 40 MHz
    Flash size:        4MB
    Features:          WiFi 6, BT 5
    MAC address:       40:4c:ca:5b:c7:dc
    App/part. size:    768,944/4,128,768 bytes, 18.62%
    [2025-04-10T10:01:23Z INFO ] Segment at address '0x0' has not changed, skipping write
    [2025-04-10T10:01:23Z INFO ] Segment at address '0x8000' has not changed, skipping write
    [00:00:06] [========================================]     411/411     0x10000                                                                                             [2025-04-10T10:01:31Z INFO ] Flashing has completed!
    ```

    If espflash reports the following error:

    ```
    Error: espflash::connection_failed

    × Error while connecting to device
    ╰─▶ Serial port not found
    ```

    You should to identify which serial port are connected to your esp board, and use `--port` to specify the used serial port:

    ```
    # Suppose that the esp board are connected to /dev/cu.usbmodem211401
    cargo run --release -- --port /dev/cu.usbmodem211401
    ```

    If you want to get some insight of segments of your binary, [`espsegs`](https://github.com/bjoernQ/espsegs) would help:

    ```
    # Install it first
    cargo install --git https://github.com/bjoernQ/espsegs

    # Check all segments
    espsegs target/xtensa-esp32s3-none-elf/release/rmk-esp32s3 --chip esp32s3
    ```

</details>

<details>
    <summary>Building using Docker with devcontainer</summary>
    To build using docker you should have all docker stack installed and the devcontainer extension in your IDE (I tested in vscode and zed)

    First start development in your devcontainer and then run:
    ```
        cargo make all
    ```
    It should run all necessary steps to build and convert the binary to .bin, which will be available in the generated folder.

</details>
