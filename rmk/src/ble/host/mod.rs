//! BLE transport adapters for the active host configurator protocol.

#[cfg(feature = "rynk")]
mod rynk;
#[cfg(feature = "vial")]
mod vial;

#[cfg(feature = "rynk")]
pub(crate) type HostConnection<'a> = rynk::HostConnection<'a>;
#[cfg(feature = "vial")]
pub(crate) type HostConnection<'a> = vial::HostConnection<'a>;

#[cfg(feature = "rynk")]
pub(crate) const HOST_WRITE_BUFFER_SIZE: usize = rmk_types::protocol::rynk::RYNK_BLE_CHUNK_SIZE;
#[cfg(feature = "vial")]
pub(crate) const HOST_WRITE_BUFFER_SIZE: usize = 32;
