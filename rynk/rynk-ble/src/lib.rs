//! BLE GATT transport using `bluest`.
//!
//! A Rynk keyboard is identified by its service UUID (`RYNK_SERVICE_UUID`), not its
//! user-settable BLE name — the counterpart to the serial transport's serial marker.
//! [`BleDevice::discover`] lists already-connected devices exposing that service (no
//! scan, no attach); [`RynkDevice::connect`] then attaches and handshakes.

use std::time::Duration;

use async_stream::stream;
use bluest::{Adapter, Characteristic, Device, DeviceId, Uuid};
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use rynk::io::{ErrorKind, Read, Write};
use rynk::rmk_types::protocol::rynk::RYNK_BLE_CHUNK_SIZE;
use rynk::{RynkDevice, RynkHostError};

const RYNK_SERVICE_UUID: Uuid = Uuid::from_u128(rynk::rmk_types::protocol::rynk::RYNK_SERVICE_UUID);
const RYNK_INPUT_CHAR_UUID: Uuid = Uuid::from_u128(rynk::rmk_types::protocol::rynk::RYNK_INPUT_CHAR_UUID);
const RYNK_OUTPUT_CHAR_UUID: Uuid = Uuid::from_u128(rynk::rmk_types::protocol::rynk::RYNK_OUTPUT_CHAR_UUID);

/// Minimal DFU GATT service for split peripherals.
const DFU_SERVICE_UUID: Uuid = Uuid::from_u128(0x4dd5fbaa_18e5_4b07_bf0a_353698659947);
const DFU_INPUT_CHAR_UUID: Uuid = Uuid::from_u128(0x0e6313e3_bd0b_45c2_8d2e_37a2e8128bc4);
const DFU_OUTPUT_CHAR_UUID: Uuid = Uuid::from_u128(0x4b3514fb_cae4_4d38_a097_3a2a3d1c3b9d);

/// Bounds connection, discovery, and subscription; those operations carry no
/// inherent timeout, so a radio-silent device would otherwise pend forever.
const GATT_TIMEOUT: Duration = Duration::from_secs(10);

/// ATT-minimum MTU payload.
const BLE_SAFE_WRITE: usize = 20;

/// Read half of the attached Rynk GATT link: an async generator that owns the
/// input characteristic and yields each notification chunk. The `notify()`
/// borrow stays inside this one pinned state machine, so there is no
/// self-referential struct, no leak, and no task; dropping it unsubscribes
/// (bluest's guard runs) and frees the characteristic.
pub struct BleReader {
    input: BoxStream<'static, std::io::Result<Vec<u8>>>,
    /// Holds a notification chunk larger than one `read` buffer across reads.
    pending: Vec<u8>,
    /// Held so the GATT connection (owned by the central) outlives the session.
    _adapter: Adapter,
}

impl rynk::io::ErrorType for BleReader {
    type Error = std::io::Error;
}

impl Read for BleReader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        while self.pending.is_empty() {
            match self.input.next().await {
                Some(Ok(chunk)) => self.pending = chunk,
                Some(Err(err)) => return Err(err),
                // Unsubscribe ends the stream → EOF → Disconnected.
                None => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

/// Write half of the attached Rynk GATT link: write-without-response, capped
/// to the characteristic's capacity.
pub struct BleWriter {
    output: Characteristic,
    write_chunk: usize,
}

impl rynk::io::ErrorType for BleWriter {
    type Error = std::io::Error;
}

impl Write for BleWriter {
    /// One GATT write per call, capped to the characteristic; `write_all` loops the
    /// rest. Write-without-response: the LE link layer still delivers reliably, and
    /// skipping the ATT ack saves a full connection-interval round trip per chunk.
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(self.write_chunk);
        self.output.write_without_response(&buf[..n]).await.map_err(|e| {
            // Preserve the GATT error detail before the driver reduces it to `ErrorKind`.
            log::warn!("rynk-ble: gatt write: {e}");
            std::io::Error::other("gatt write")
        })?;
        Ok(n)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// A discovered Rynk keyboard, for a device picker. Holds cheap `bluest` handles,
/// not a live session; `connect` performs the first attach.
pub struct BleDevice {
    /// The keyboard's BLE name, if it advertised one.
    pub name: Option<String>,
    adapter: Adapter,
    device: Device,
}

impl BleDevice {
    /// Stable picker key — unlike the BLE name, which may be absent or shared.
    pub fn id(&self) -> DeviceId {
        self.device.id()
    }

    /// List already-connected Rynk keyboards (those exposing the service) — no scan,
    /// no attach. Requires Bluetooth permission; a denied/off adapter hangs in
    /// `wait_available` rather than erroring. Discovery is transport-specific, so
    /// it's an inherent call, not part of [`RynkDevice`].
    pub async fn discover() -> Result<Vec<Self>, RynkHostError> {
        let adapter = Adapter::default()
            .await
            .ok_or_else(|| RynkHostError::DeviceNotFound("no BLE adapter".into()))?;
        adapter
            .wait_available()
            .await
            .map_err(|e| RynkHostError::Transport("wait_available", e.to_string()))?;

        let connected = adapter
            .connected_devices_with_services(&[RYNK_SERVICE_UUID])
            .await
            .map_err(|e| RynkHostError::Transport("connected_devices_with_services", e.to_string()))?;
        let mut devices = Vec::with_capacity(connected.len());
        for device in connected {
            devices.push(BleDevice {
                name: device.name_async().await.ok(),
                adapter: adapter.clone(),
                device,
            });
        }
        Ok(devices)
    }

    /// Scan for all advertising BLE devices (connected + new), not limited to
    /// the Rynk service. Used by rynk-wtf to find peripherals in DFU mode
    /// which advertise with a name like `"rmk per0"`.
    pub async fn discover_all() -> Result<Vec<Self>, RynkHostError> {
        let adapter = Adapter::default()
            .await
            .ok_or_else(|| RynkHostError::DeviceNotFound("no BLE adapter".into()))?;
        adapter
            .wait_available()
            .await
            .map_err(|e| RynkHostError::Transport("wait_available", e.to_string()))?;

        // First include already-connected devices.
        let connected = adapter
            .connected_devices()
            .await
            .map_err(|e| RynkHostError::Transport("connected_devices", e.to_string()))?;
        let mut seen = std::collections::HashSet::new();
        let mut devices = Vec::new();
        for device in connected {
            let name = device.name_async().await.ok();
            let id = device.id();
            seen.insert(id.clone());
            devices.push(BleDevice {
                name,
                adapter: adapter.clone(),
                device,
            });
        }

        // Then scan for new advertising devices for a short window.
        use futures_util::StreamExt;
        let scan = adapter
            .scan(&[])
            .await
            .map_err(|e| RynkHostError::Transport("scan", e.to_string()))?;
        futures_util::pin_mut!(scan);
        let scan_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while let Some(adv) = tokio::time::timeout_at(tokio::time::Instant::from_std(scan_deadline), scan.next())
            .await
            .unwrap_or(None)
        {
            let device = adv.device;
            let id = device.id();
            if seen.insert(id) {
                let name = device.name_async().await.ok();
                devices.push(BleDevice {
                    name,
                    adapter: adapter.clone(),
                    device,
                });
            }
        }

        Ok(devices)
    }

    // Discover the Rynk service and its input/output characteristics.
    // Falls back to the DFU service for split peripherals in DFU mode.
    async fn discover_characteristic(&self) -> Result<(Characteristic, Characteristic), RynkHostError> {
        // Try Rynk service first (full keyboard).
        if let Ok(Some((input, output))) = self
            .try_discover_service(RYNK_SERVICE_UUID, RYNK_INPUT_CHAR_UUID, RYNK_OUTPUT_CHAR_UUID)
            .await
        {
            return Ok((input, output));
        }
        // Fall back to DFU service (split peripheral in DFU mode).
        self.try_discover_service(DFU_SERVICE_UUID, DFU_INPUT_CHAR_UUID, DFU_OUTPUT_CHAR_UUID)
            .await
            .map_err(|_| RynkHostError::DeviceNotFound("neither Rynk nor DFU GATT service found".into()))?
            .ok_or_else(|| RynkHostError::DeviceNotFound("neither Rynk nor DFU GATT service found".into()))
    }

    async fn try_discover_service(
        &self,
        service_uuid: Uuid,
        input_uuid: Uuid,
        output_uuid: Uuid,
    ) -> Result<Option<(Characteristic, Characteristic)>, RynkHostError> {
        let services = self
            .device
            .discover_services_with_uuid(service_uuid)
            .await
            .map_err(|e| RynkHostError::Transport("discover_services", e.to_string()))?;
        let service = match services.into_iter().next() {
            Some(s) => s,
            None => return Ok(None),
        };
        let mut input_char = None;
        let mut output_char = None;
        for c in service
            .discover_characteristics()
            .await
            .map_err(|e| RynkHostError::Transport("discover_characteristics", e.to_string()))?
        {
            match c
                .uuid_async()
                .await
                .map_err(|e| RynkHostError::Transport("characteristic uuid", e.to_string()))?
            {
                u if u == input_uuid => input_char = Some(c),
                u if u == output_uuid => output_char = Some(c),
                _ => {}
            }
        }
        let input = input_char.ok_or_else(|| RynkHostError::DeviceNotFound("input characteristic missing".into()))?;
        let output =
            output_char.ok_or_else(|| RynkHostError::DeviceNotFound("output characteristic missing".into()))?;
        Ok(Some((input, output)))
    }

    /// Subscribe and build the transport. bluest's notify stream borrows the
    /// characteristic, so a generator owns `input` and `notify()`s it — keeping the
    /// borrow inside one pinned state machine (no self-reference, no leak, no task).
    /// Its synthetic empty first chunk acks that the subscription is live; consuming
    /// it here means `attach` returns only once subscribed, the order the firmware
    /// needs before the client's first write (bounded; a silent device never acks).
    async fn attach(
        self,
        input: Characteristic,
        output: Characteristic,
    ) -> Result<(BleReader, BleWriter), RynkHostError> {
        // Cap writes to the characteristic's capacity.
        let write_chunk = output
            .max_write_len_async()
            .await
            .unwrap_or(BLE_SAFE_WRITE)
            .clamp(BLE_SAFE_WRITE, RYNK_BLE_CHUNK_SIZE);

        let mut input = stream! {
            // `notify().await` returns only once the subscription is live; `input`
            // is moved into and owned by this state machine.
            let updates = match input.notify().await {
                Ok(updates) => updates,
                Err(err) => {
                    yield Err(std::io::Error::other(err));
                    return;
                }
            };
            yield Ok(Vec::new()); // readiness ack: subscription is now live
            futures_util::pin_mut!(updates);
            while let Some(update) = updates.next().await {
                yield update.map_err(std::io::Error::other);
            }
        }
        .boxed();

        // Block on the readiness ack so we return only once live. `open` bounds it.
        match input.next().await {
            Some(Ok(_)) => {}
            Some(Err(err)) => return Err(RynkHostError::Transport("notify", err.to_string())),
            None => return Err(RynkHostError::Disconnected),
        }

        Ok((
            BleReader {
                input,
                pending: Vec::new(),
                _adapter: self.adapter,
            },
            BleWriter { output, write_chunk },
        ))
    }
}

impl RynkDevice for BleDevice {
    type Read = BleReader;
    type Write = BleWriter;

    fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("{:?}", self.id()))
    }

    /// Connect, discover characteristics, and subscribe — once, no retry. A failure
    /// means the device is gone or isn't a Rynk keyboard.
    async fn open(self) -> Result<(BleReader, BleWriter), RynkHostError> {
        tokio::time::timeout(GATT_TIMEOUT, async {
            self.adapter
                .connect_device(&self.device)
                .await
                .map_err(|e| RynkHostError::Transport("connect_device", e.to_string()))?;
            let (input, output) = self.discover_characteristic().await?;
            self.attach(input, output).await
        })
        .await
        .map_err(|_| RynkHostError::Io(ErrorKind::TimedOut))?
    }
}
