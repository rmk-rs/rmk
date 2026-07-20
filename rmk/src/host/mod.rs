pub(crate) mod context;
// Physical-presence unlock gate, shared by the Vial (`vial` + `host_lock`)
// and Rynk (`rynk` ⇒ `host_lock`) services.
#[cfg(feature = "host_lock")]
pub(crate) mod lock;
#[cfg(feature = "rynk")]
pub(crate) mod rynk;
#[cfg(feature = "storage")]
pub(crate) mod storage;
// Shared transport-adapter error, used by the USB/BLE Vial and USB/BLE Rynk
// adapters. Gated to exactly the feature combos that compile an adapter.
#[cfg(any(
    all(feature = "vial", not(feature = "_no_usb")),
    all(feature = "vial", feature = "_ble"),
    all(feature = "rynk", not(feature = "_no_usb")),
    all(feature = "rynk", feature = "_ble"),
))]
pub(crate) mod transport;
#[cfg(feature = "vial")]
pub(crate) mod via;

/// The active host-protocol service. Resolves to [`via::VialService`]
/// under the `vial` feature and [`rynk::RynkService`] under `rynk` (the
/// two are mutually exclusive).
#[cfg(feature = "rynk")]
pub use rynk::RynkService as HostService;
#[cfg(all(feature = "rynk", feature = "lighting"))]
pub use rynk::{
    RYNK_LIGHTING_TRANSACTION_CAPACITY, RynkLightingController, RynkLightingDescriptor, RynkLightingMailbox,
    StandardRynkLightingAdapter,
};
/// RMK's semantic version, available to downstream firmware build labels.
#[cfg(feature = "rynk")]
pub use rynk::{RMK_VERSION, RMK_VERSION_STRING};
#[cfg(feature = "vial")]
pub use via::VialService as HostService;
