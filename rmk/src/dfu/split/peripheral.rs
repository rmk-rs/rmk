use core::sync::atomic::Ordering;

/// Return the CRC-32 of the currently running firmware binary.
///
/// The result is computed once and cached.  The firmware region is
/// determined by the `__vector_table` / `__veneer_limit` linker symbols,
/// covering the entire `.text` + `.rodata` + `.data` sections.
pub fn read_embedded_firmware_hash() -> u32 {
    use core::sync::atomic::AtomicU32;
    static CACHED_HASH: AtomicU32 = AtomicU32::new(u32::MAX);
    let cached = CACHED_HASH.load(Ordering::Acquire);
    if cached != u32::MAX {
        return cached;
    }
    unsafe extern "C" {
        static __vector_table: u8;
        static __veneer_limit: u8;
    }
    let start = unsafe { &__vector_table as *const u8 };
    let end = unsafe { &__veneer_limit as *const u8 };
    let len = end as usize - start as usize;
    let data = unsafe { core::slice::from_raw_parts(start, len) };
    let hash = crate::crc32::crc32(data);
    CACHED_HASH.store(hash, Ordering::Release);
    hash
}
