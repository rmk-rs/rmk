# rp2040 USB DFU Split

A split keyboard example where **both** the central and the peripheral have their own USB DFU interface.

## How it works

Each side exposes a USB DFU device so firmware can be updated independently:

- **Central** — runs a full RMK keyboard with `UsbTransport::new()`. The DFU interface is part of the USB transport.
- **Peripheral** — runs `run_peripheral_usb_with_dfu()`, giving it its own USB DFU device alongside the UART split link.

Flashing each side is done through its own USB port — no firmware forwarding through the central is needed.

## Build

```bash
# Central
cargo build --release --bin central

# Peripheral
cargo build --release --bin peripheral
```
