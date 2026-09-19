/// CRC-32 lookup table (IEEE 802.3 / PKZip), computed at compile time.
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// Incremental CRC-32 (IEEE 802.3 / PKZip) calculator.
///
/// Uses polynomial `0xEDB88320` (reflected `0x04C11DB7`), the same as
/// Ethernet, PKZip, and `crc32` on POSIX.  `no_std` compatible — the
/// lookup table is pre‑computed at compile time.
///
/// # Example
///
/// ```rust
/// let mut c = Crc32::new();
/// c.update(b"hello ");
/// c.update(b"world");
/// assert_eq!(c.finalize(), 0x0B4C_4A99);
/// ```
pub struct Crc32 {
    state: u32,
}

impl Crc32 {
    /// Create a new CRC-32 calculator initialised to `0xFFFF_FFFF`.
    pub const fn new() -> Self {
        Self { state: !0u32 }
    }

    /// Feed a chunk of data into the running CRC.
    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let idx = ((self.state ^ byte as u32) & 0xFF) as usize;
            self.state = (self.state >> 8) ^ CRC32_TABLE[idx];
        }
    }

    /// Finalize and return the CRC-32 (XOR‑out with `0xFFFF_FFFF`).
    pub const fn finalize(&self) -> u32 {
        !self.state
    }

    /// Restore a CRC calculator from a previously finalized value.
    ///
    /// This is the inverse of [`finalize`](Self::finalize): since
    /// `finalize()` returns `!self.state`, the internal state is recovered as
    /// `!finalized`. Useful for checkpoint/rollback in DFU transfer
    /// verification.
    pub const fn from_state(finalized: u32) -> Self {
        Self { state: !finalized }
    }
}

/// Compute the CRC-32 of `data` in one shot.
///
/// Equivalent to:
/// ```rust
/// let mut c = Crc32::new();
/// c.update(data);
/// c.finalize()
/// ```
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(data);
    c.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    // test reference values from https://crccalc.com/ CRC-32/ISO-HDLC
    #[test]
    fn test_empty() {
        assert_eq!(crc32(b""), 0x0000_0000);
    }

    #[test]
    fn test_hello_world() {
        assert_eq!(crc32(b"hello world"), 0x0D4A_1185);
    }

    #[test]
    fn test_incremental_vs_oneshot() {
        let data = b"The quick brown fox jumps over the lazy dog";
        assert_eq!(crc32(data), 0x414F_A339);

        let mut c = Crc32::new();
        c.update(b"The quick brown ");
        c.update(b"fox jumps over ");
        c.update(b"the lazy dog");
        assert_eq!(c.finalize(), 0x414F_A339);
    }

    #[test]
    fn test_all_zeros_32() {
        let mut c = Crc32::new();
        c.update(&[0u8; 32]);
        assert_eq!(c.finalize(), 0x190A_55AD);
    }

    #[test]
    fn test_all_ffs_32() {
        let mut c = Crc32::new();
        c.update(&[0xFFu8; 32]);
        assert_eq!(c.finalize(), 0xFF6C_AB0B);
    }

    #[test]
    fn test_known_values() {
        // CRC32 of 4 consecutive bytes 0..4
        assert_eq!(crc32(&[0, 1, 2, 3]), 0x8BB9_8613);
        // CRC32 of "123456789" (standard test vector)
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn test_new_is_idempotent() {
        let c1 = Crc32::new();
        let c2 = Crc32::new();
        assert_eq!(c1.finalize(), c2.finalize());
    }
}
