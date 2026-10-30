//! BLE transport adapters for the active host configurator protocol.

#[cfg(feature = "rynk")]
mod rynk;
#[cfg(feature = "vial")]
mod vial;

#[cfg(feature = "rynk")]
pub(crate) type HostGattHandler<'r, 'a> = rynk::HostGattHandler<'r, 'a>;
#[cfg(feature = "vial")]
pub(crate) type HostGattHandler<'r, 'a> = vial::HostGattHandler<'r, 'a>;

#[cfg(feature = "rynk")]
pub(crate) type BleConfig<'a> = rynk::BleConfig<'a>;
#[cfg(feature = "vial")]
pub(crate) type BleConfig<'a> = vial::BleConfig<'a>;

#[cfg(feature = "rynk")]
pub(crate) const HOST_WRITE_BUFFER_SIZE: usize = rmk_types::protocol::rynk::RYNK_BLE_CHUNK_SIZE;
#[cfg(feature = "vial")]
pub(crate) const HOST_WRITE_BUFFER_SIZE: usize = 32;

/// Result of dispatching a GATT write to the active host protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostWriteOutcome {
    /// The protocol consumed the write or rejected its payload.
    Handled,
    /// A notification subscription changed and the bonded CCCD table must be saved.
    CccdUpdated,
    /// An HID control-point write should use the common suspend/resume handling.
    ControlPoint,
    /// The handle does not belong to the active host protocol.
    Unhandled,
}
