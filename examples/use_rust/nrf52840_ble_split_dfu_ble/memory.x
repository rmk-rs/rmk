MEMORY {
  FLASH : ORIGIN = 0x00007000, LENGTH = 491520   /* ACTIVE region */
  RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}

__bootloader_state_start   = 0x6000;
__bootloader_state_end     = 0x7000;
__bootloader_active_start  = 0x7000;
__bootloader_active_end    = 0x7F000;
__bootloader_dfu_start     = 0x7F000;
__bootloader_dfu_end       = 0xF8000;
__bootloader_storage_start = 0xF8000;
__bootloader_storage_end   = 0x100000;
