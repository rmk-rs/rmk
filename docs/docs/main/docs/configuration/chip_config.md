# Chip-Specific Configuration

The `[chip.<chip_name>]` section allows you to configure chip-specific settings for different microcontrollers. These settings are optional and only needed when you want to override the default behavior.

## nRF52840

### DCDC Configuration

The nRF52840 has two DC/DC regulators that can improve power efficiency when enabled with proper hardware support.

#### Configuration Example

```toml
[chip.nrf52840]
# Enable DCDC regulator 0
dcdc_reg0 = true
# Enable DCDC regulator 1
dcdc_reg1 = true
# Set DCDC regulator 0 voltage
dcdc_reg0_voltage = "3V3"  # Options: "3V3" or "1V8"
```

::: danger Hardware Requirement
Do not enable DC/DC regulators without an external LC filter being connected, as this will inhibit device operation, including debug access, until an LC filter is connected.
:::
::: danger Bootloader Version
If your hardware is using the [Adafruit nRF52 Bootloader](https://github.com/adafruit/Adafruit_nRF52_Bootloader) (e.g. nice!nano) ensure that your bootloader is updated to version ≥ [0.10.0](https://github.com/adafruit/Adafruit_nRF52_Bootloader/releases/tag/0.10.0) when setting a voltage other than 3V3.
There is a bug in older versions that can lead to a boot loop which can only be fixed by re-flashing the bootloader via the debug interface.
:::

## nRF52833

### DCDC Configuration

The nRF52833 has one DC/DC regulator available for configuration. Its REG0 stage has no DC/DC mode, but when the chip is supplied through VDDH you can set REG0's output voltage, which is VDD and the GPIO level.

#### Configuration Example

```toml
[chip.nrf52833]
# Enable DCDC regulator 1
dcdc_reg1 = true
# REG0 output voltage (VDD) when supplied through VDDH, stored in UICR.REGOUT0.
dcdc_reg0_voltage = "3V3"  # Options: "3V3" or "1V8"
```

::: danger Hardware Requirement
Do not enable DC/DC regulator without an external LC filter being connected, as this will inhibit device operation, including debug access, until an LC filter is connected.
:::
::: danger Bootloader Version
If your hardware is using the [Adafruit nRF52 Bootloader](https://github.com/adafruit/Adafruit_nRF52_Bootloader) (e.g. nice!nano) ensure that your bootloader is updated to version ≥ [0.10.0](https://github.com/adafruit/Adafruit_nRF52_Bootloader/releases/tag/0.10.0) when setting a voltage other than 3V3.
There is a bug in older versions that can lead to a boot loop which can only be fixed by re-flashing the bootloader via the debug interface.
:::
