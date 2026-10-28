/// Config for storage
#[derive(Clone, Copy, Debug)]
pub struct StorageConfig {
    /// Start address of local storage, MUST BE start of a sector.
    /// Zero selects the last `num_sectors` sectors, except on nRF BLE builds
    /// without DFU, where it selects `0x60000`.
    pub start_addr: usize,
    // Number of sectors used for storage, >= 2.
    pub num_sectors: u8,
    pub clear_storage: bool,
    pub clear_layout: bool,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            start_addr: 0,
            num_sectors: 2,
            clear_storage: false,
            clear_layout: false,
        }
    }
}
