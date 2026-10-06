pub(crate) mod context;
// Physical-presence unlock gate, shared by the Vial (`vial` + `host_lock`)
// and Rynk (`rynk` ⇒ `host_lock`) services.
#[cfg(feature = "host_lock")]
pub(crate) mod lock;
#[cfg(feature = "rynk")]
pub(crate) mod rynk;
#[cfg(feature = "storage")]
pub(crate) mod storage;
// Shared transport-adapter error, used by the USB/BLE Vial and BLE Rynk
// adapters. Gated to exactly the feature combos that compile an adapter.
#[cfg(any(
    all(feature = "vial", not(feature = "_no_usb")),
    all(feature = "vial", feature = "_ble"),
    all(feature = "rynk", feature = "_ble"),
))]
pub(crate) mod transport;
#[cfg(feature = "vial")]
pub(crate) mod via;

/// The active host-protocol service, selected by the mutually exclusive
/// `vial` and `rynk` features.
#[cfg(feature = "rynk")]
pub use rynk::RynkService as HostService;
#[cfg(feature = "vial")]
pub use via::VialService as HostService;
