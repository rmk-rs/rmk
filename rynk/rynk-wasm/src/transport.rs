//! The JS-owned byte link as a [`RynkDevice`], plus the `rynk::io::Read`/`Write`
//! halves its `open()` hands out. The page owns the link's lifetime: it opens
//! the link before `connect` and closes it on teardown — nothing here closes it.

use js_sys::{Promise, Uint8Array};
use rynk::io::{ErrorKind, ErrorType, Read, Write};
use rynk::{RynkDevice, RynkHostError};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    /// JS byte link. `recv()` returns bytes, or an empty array only at EOF.
    #[derive(Clone)]
    pub type JsByteLink;

    // Distinct Rust name: a same-named inherent `label` would shadow the trait
    // method below, whose forward would otherwise read as self-recursion.
    #[wasm_bindgen(method, getter, js_name = label)]
    fn js_label(this: &JsByteLink) -> String;

    /// Raw `Promise` imports (not `async`) so the halves get nameable futures
    /// they can park across cancelled `read`s and `write`s.
    #[wasm_bindgen(method, catch)]
    fn send(this: &JsByteLink, frame: Uint8Array) -> Result<Promise, JsValue>;

    #[wasm_bindgen(method, catch)]
    fn recv(this: &JsByteLink) -> Result<Promise, JsValue>;
}

/// The browser owns discovery (WebSerial/WebHID chooser) and opens the link, so
/// the already-open [`JsByteLink`] itself is the transport's [`RynkDevice`]:
/// `open()` only wraps it into halves, and the trait's `connect()` handshakes.
impl RynkDevice for JsByteLink {
    type Read = WasmReader;
    type Write = WasmWriter;

    fn label(&self) -> String {
        self.js_label()
    }

    async fn open(self) -> Result<(WasmReader, WasmWriter), RynkHostError> {
        Ok((
            WasmReader {
                link: self.clone(),
                recv: None,
                pending: Vec::new(),
                pos: 0,
            },
            WasmWriter { link: self, send: None },
        ))
    }
}

/// Read half of the JS byte link, buffering `recv()` chunks.
pub struct WasmReader {
    link: JsByteLink,
    /// In-flight `recv()`, parked so a cancelled `read` resumes it.
    recv: Option<JsFuture>,
    /// Holds a chunk larger than one `read` buffer across reads.
    pending: Vec<u8>,
    pos: usize,
}

impl ErrorType for WasmReader {
    type Error = ErrorKind;
}

impl Read for WasmReader {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        // Refill once the current chunk is drained.
        while self.pos >= self.pending.len() {
            if self.recv.is_none() {
                self.recv = Some(JsFuture::from(self.link.recv().map_err(|_| ErrorKind::Other)?));
            }
            let value = self.recv.as_mut().unwrap().await;
            self.recv = None;
            let value = value.map_err(|_| ErrorKind::Other)?;
            // Only an empty byte array is EOF; any other JS value is invalid data.
            let chunk = value.dyn_into::<Uint8Array>().map_err(|_| ErrorKind::InvalidData)?;
            if chunk.length() == 0 {
                return Ok(0); // EOF
            }
            self.pending = chunk.to_vec();
            self.pos = 0;
        }
        let n = buf.len().min(self.pending.len() - self.pos);
        buf[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Write half of the JS byte link.
pub struct WasmWriter {
    link: JsByteLink,
    /// In-flight `send()` and its length, parked so the driver's retry of a
    /// cancelled `write` finishes it instead of sending the bytes again: JS
    /// promises do not cancel.
    send: Option<(JsFuture, usize)>,
}

impl ErrorType for WasmWriter {
    type Error = ErrorKind;
}

impl Write for WasmWriter {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.send.is_none() {
            let send = self.link.send(Uint8Array::from(buf)).map_err(|_| ErrorKind::Other)?;
            self.send = Some((JsFuture::from(send), buf.len()));
        }
        let (send, len) = self.send.as_mut().unwrap();
        let done = send.await;
        let len = *len;
        // Cleared only once the send resolves, so a cancel above re-parks it.
        self.send = None;
        done.map_err(|_| ErrorKind::Other)?;
        Ok(len)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::future::ready;
    use std::rc::Rc;

    use embassy_futures::select::select;
    use wasm_bindgen_futures::future_to_promise;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    /// Mirrors the page's WebHID link, which walks a frame across awaited
    /// `sendReport` calls — one byte per report here, `N` per report in `index.html`.
    #[wasm_bindgen]
    struct FakeWebHidLink {
        sent: Rc<RefCell<String>>,
    }

    #[wasm_bindgen]
    impl FakeWebHidLink {
        pub fn send(&self, frame: Uint8Array) -> Promise {
            let sent = self.sent.clone();
            let frame = frame.to_vec();
            future_to_promise(async move {
                for byte in frame {
                    let send_report = JsFuture::from(Promise::resolve(&JsValue::UNDEFINED));
                    send_report.await?;
                    sent.borrow_mut().push(byte as char);
                }
                Ok(JsValue::UNDEFINED)
            })
        }
    }

    /// The wasm pump drops `Driver::run` — mid-frame — whenever a call it was
    /// carrying resolves, and the next run retries that write. Cancelling cannot
    /// stop the JS `send` it had started, so the retry must finish that send
    /// rather than put the frame on the wire a second time.
    #[wasm_bindgen_test]
    async fn write_is_cancel_safe() {
        let sent = Rc::new(RefCell::new(String::new()));
        let link: JsByteLink = JsValue::from(FakeWebHidLink { sent: sent.clone() }).unchecked_into();
        let mut writer = WasmWriter { link, send: None };

        // Two bytes per frame: a one-byte frame goes out in a single report, with
        // no gap inside it for the next send to slip into.
        let cancel_immediately = ready(());
        select(writer.write(b"AB"), cancel_immediately).await;
        assert_eq!(writer.write(b"AB").await.unwrap(), 2);
        writer.write(b"CD").await.unwrap();

        assert_eq!(sent.take(), "ABCD");
    }
}
