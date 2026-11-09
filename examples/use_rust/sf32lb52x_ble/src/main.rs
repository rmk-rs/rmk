#![no_main]
#![no_std]

#[macro_use]
mod keymap;
#[macro_use]
mod macros;
mod display;
mod renderers;
mod vial;

use defmt::{error, info};
use defmt_rtt as _;
use display::{AlignedFb, LCD_H, LCD_W, LcdcBus, LockingLcdcInterface, power_cycle};
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Delay;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::WebColors;
use keymap::{COL, ROW, SIZE};
use lcd_async::Builder;
use lcd_async::models::GC9107;
use panic_probe as _;
use rand_chacha::ChaCha12Rng;
use rand_core::SeedableRng;
use renderers::KeyLabelRenderer;
use rmk::ble::{BleTransport, build_ble_stack};
use rmk::config::{BehaviorConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::display::DisplayProcessor;
use rmk::display::drivers::lcd_async::LcdAsyncDisplay;
use rmk::host::HostService;
use rmk::input_device::rotary_encoder::RotaryEncoder;
use rmk::keyboard::Keyboard;
use rmk::matrix::direct_pin::DirectPinMatrix;
use rmk::processor::builtin::wpm::WpmProcessor;
use rmk::storage::async_flash_wrapper;
use rmk::usb::UsbTransport;
use rmk::{HostResources, KeymapData, initialize_keymap_and_storage, run_all};
use sifli_hal::efuse::Efuse;
use sifli_hal::gpio::{Input, Level, Output};
use sifli_hal::mpi::{BlockingNorFlash, BuiltInProfile, NorConfig, ProfileSource};
use sifli_hal::peripherals::{EFUSEC, USBC};
use sifli_hal::rng::Rng;
use sifli_hal::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use sifli_hal::{bind_interrupts, ipc, pmu};
use sifli_radio::bluetooth::{BleController, BleInitConfig};
use static_cell::StaticCell;
use vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};

/// Background colour per display, in CS-pin order
/// (idx 0 → PA03/CS1, idx 1 → PA02/CS2, idx 2 → PA01/CS3).
const PALETTE: [Rgb565; 3] = [Rgb565::CSS_DARK_RED, Rgb565::CSS_DARK_GREEN, Rgb565::CSS_DARK_BLUE];

bind_interrupts!(struct Irqs {
    MAILBOX2_CH1 => ipc::InterruptHandler;
    USBC => UsbInterruptHandler<USBC>;
});

/// Apply factory PMU trims and get a stable BLE static random address.
fn ble_addr(efusec: EFUSEC) -> [u8; 6] {
    let mut addr = [0u8; 6];
    match Efuse::new(efusec) {
        Ok(efuse) => {
            let applied = pmu::apply_calibration(efuse.calibration());
            info!("PMU calibration applied from eFUSE: {}", applied);
            // First 6 bytes of UID → BD_ADDR (little-endian byte order matches
            // the HCI Set_Random_Address command).
            addr.copy_from_slice(&efuse.uid().bytes()[..6]);
        }
        Err(e) => {
            error!("Efuse init failed, PMU trims left at defaults: {:?}", e);
            // Use fixed ble addr if Efuse init failed
            addr = [0x18, 0xe2, 0x21, 0x80, 0xc0, 0xc7]
        }
    }
    addr[5] |= 0xC0; // top two bits = 11 → Random Static
    addr
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Hello World! RMK SF32LB52x BLE example");
    let p = sifli_hal::init(sifli_hal::Config::default());

    sifli_hal::rcc::test_print_clocks();

    // Get BLE addr
    let ble_addr = ble_addr(p.EFUSEC);
    info!(
        "Using BLE address {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        ble_addr[5], ble_addr[4], ble_addr[3], ble_addr[2], ble_addr[1], ble_addr[0],
    );

    // Give probe-rs time to attach before touching the BLE stack.
    embassy_time::Timer::after_millis(500).await;

    // Enable MPI2 NOR while running from its XIP window.
    let blocking_flash = match BlockingNorFlash::new_blocking_without_reset(
        p.MPI2,
        ProfileSource::BuiltIn(BuiltInProfile::CommonSpiNor16MiB),
        NorConfig::default(),
    ) {
        Ok(f) => f,
        Err(e) => {
            error!("MPI2 NOR flash init failed: {:?}", e);
            loop {
                embassy_time::Timer::after_secs(1).await;
            }
        }
    };

    let async_flash = async_flash_wrapper(blocking_flash);

    let storage_config = StorageConfig {
        num_sectors: 16,
        ..Default::default()
    };
    let mut keymap_data = KeymapData::new_with_encoder(keymap::get_default_keymap(), keymap::get_default_encoder_map());
    let mut behavior_config = BehaviorConfig::default();
    let per_key_config = PositionalConfig::default();
    let (keymap, mut storage) = initialize_keymap_and_storage(
        &mut keymap_data,
        async_flash,
        &storage_config,
        &mut behavior_config,
        &per_key_config,
    )
    .await;

    // Initialize BLE controller (LCPU + IPC + HCI).
    let controller: BleController = match BleController::new(
        p.LCPU,
        p.MAILBOX1_CH1,
        p.DMAC2_CH8,
        Irqs,
        &BleInitConfig::default().pm_enabled(false),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            error!("BLE init failed: {:?}", e);
            loop {
                embassy_time::Timer::after_secs(1).await;
            }
        }
    };
    let mut rng = Rng::new_blocking(p.TRNG);
    let mut rng_gen = ChaCha12Rng::from_rng(&mut rng).unwrap();
    let mut host_resources = HostResources::new();
    let stack = build_ble_stack(controller, ble_addr, &mut rng_gen, &mut host_resources).await;

    // Initialize USB driver (dual-mode USB + BLE). PA35/PA36 are the USB D+/D- pins.
    let usb_driver = Driver::new(p.USBC, Irqs, p.PA35, p.PA36);

    // Pin config: SF32 SuperKey macro keyboard (https://github.com/SiFliSparks/SuperKey)
    // KEY1=PA26, KEY2=PA33, KEY3=PA32, KEY4=PA40 — active-low with pull-ups.
    let direct_pins = config_direct_pins_sifli!(peripherals: p, direct_pins: [[PA26, PA33, PA32, PA40]]);

    let keyboard_device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x4644,
        manufacturer: "RMK & SiFli-rs",
        product_name: "RMK SF32LB52",
        serial_number: "000002",
    };

    let vial_config = VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF, &[(0, 0), (0, 3)]);

    let rmk_config = RmkConfig {
        device_config: keyboard_device_config,
        vial_config,
        storage_config,
        ..Default::default()
    };

    let debouncer = DefaultDebouncer::new();
    let mut matrix = DirectPinMatrix::<_, _, ROW, COL, SIZE>::new(direct_pins, debouncer, true);
    let mut keyboard = Keyboard::new(&keymap);
    let host_ctx = rmk::host::KeyboardContext::new(&keymap);
    let mut host_service = HostService::new(&host_ctx, &rmk_config);

    // Rotary encoder: PA43 = phase A, PA41 = phase B. Detents short to GND, so pull-ups are required.
    // Resolution 4 collapses the 4 quadrature transitions per detent into a single event;
    // reverse = true matches the SuperKey's mechanical orientation.
    let encoder_a = Input::new(p.PA43, sifli_hal::gpio::Pull::Up);
    let encoder_b = Input::new(p.PA41, sifli_hal::gpio::Pull::Up);
    let mut encoder = RotaryEncoder::with_resolution(encoder_a, encoder_b, 4, true, 0).with_debounce(2);

    // ── 3× GC9107 displays (SuperKey 3-screen module) ──
    // PA00 RST · PA01/PA02/PA03 CS · PA04/PA05/PA06 LCDC1 SPI · PA07 3V3_EN.
    info!("Initializing displays...");
    let _lcd_power = Output::new(p.PA7, Level::High);
    let mut lcd_rst = Output::new(p.PA0, Level::High);
    let lcd_cs0 = Output::new(p.PA3, Level::High); // CS1 — leftmost screen
    let lcd_cs1 = Output::new(p.PA2, Level::High); // CS2 — middle screen
    let lcd_cs2 = Output::new(p.PA1, Level::High); // CS3 — rightmost screen
    let lcdc_bus = LcdcBus::new(p.LCDC1, p.PA4, p.PA5, p.PA6);

    // One reset pulse for all 3 panels at once; the per-panel `Builder::init`
    // below runs each chip's init sequence sequentially under the bus mutex.
    power_cycle(&mut lcd_rst).await;

    static SHARED_BUS: StaticCell<Mutex<NoopRawMutex, LcdcBus>> = StaticCell::new();
    let bus = SHARED_BUS.init(Mutex::new(lcdc_bus));

    static FB0: StaticCell<AlignedFb> = StaticCell::new();
    static FB1: StaticCell<AlignedFb> = StaticCell::new();
    static FB2: StaticCell<AlignedFb> = StaticCell::new();
    let fb0 = FB0.init(AlignedFb::new());
    let fb1 = FB1.init(AlignedFb::new());
    let fb2 = FB2.init(AlignedFb::new());

    // GC9107's framebuffer is 128×160; the SuperKey panel only exposes a
    // 128×128 viewport, mapped to the bottom of the framebuffer (chip rows
    // 32..159). `display_offset(0, 32)` shifts CASET/RASET so writes land in
    // the visible region.
    let display0 = Builder::new(GC9107, LockingLcdcInterface::new(bus, lcd_cs0))
        .display_size(LCD_W, LCD_H)
        .display_offset(0, 32)
        .init(&mut Delay)
        .await
        .unwrap();
    let display1 = Builder::new(GC9107, LockingLcdcInterface::new(bus, lcd_cs1))
        .display_size(LCD_W, LCD_H)
        .display_offset(0, 32)
        .init(&mut Delay)
        .await
        .unwrap();
    let display2 = Builder::new(GC9107, LockingLcdcInterface::new(bus, lcd_cs2))
        .display_size(LCD_W, LCD_H)
        .display_offset(0, 32)
        .init(&mut Delay)
        .await
        .unwrap();

    const W: usize = LCD_W as usize;
    const H: usize = LCD_H as usize;

    let mut disp0 = DisplayProcessor::with_renderer(
        LcdAsyncDisplay::<_, _, _, _, W, H>::new(display0, fb0),
        KeyLabelRenderer::new(&keymap, 0, 0, PALETTE[0]),
    );
    let mut disp1 = DisplayProcessor::with_renderer(
        LcdAsyncDisplay::<_, _, _, _, W, H>::new(display1, fb1),
        KeyLabelRenderer::new(&keymap, 0, 1, PALETTE[1]),
    );
    let mut disp2 = DisplayProcessor::with_renderer(
        LcdAsyncDisplay::<_, _, _, _, W, H>::new(display2, fb2),
        KeyLabelRenderer::new(&keymap, 0, 2, PALETTE[2]),
    );

    info!("Starting RMK dual-mode (USB + BLE) runner...");

    let mut usb_transport = UsbTransport::new(usb_driver, rmk_config.device_config.clone());
    let mut ble_transport = BleTransport::new(&stack, rmk_config).await;
    let mut wpm_processor = WpmProcessor::new();

    run_all!(
        matrix,
        encoder,
        storage,
        disp0,
        disp1,
        disp2,
        usb_transport,
        ble_transport,
        wpm_processor,
        keyboard,
        host_service
    )
    .await;
}
