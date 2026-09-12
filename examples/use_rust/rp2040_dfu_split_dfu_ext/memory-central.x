/* rmk-memory.x for RP2040 2 MB with external-flash DFU (dfu_ext)
 *
 * Provides the MEMORY layout (absolute XIP addresses) and the standard
 * embassy-boot `__bootloader_*` partition symbols (flash-relative offsets)
 * consumed by partitions_from_linkerscript().
 *
 * With dfu_ext the DFU download slot lives on the external SPI flash, so no
 * DFU partition is carved out of the internal flash: the ACTIVE region
 * expands to fill the freed space (rounded down to a multiple of the 64K
 * swap page, matching the rmk-boot dfu_ext layout). The dfu symbols are
 * 0/unused.
 *
 * If your board has a different flash size, replace this file with the
 * matching file from the rmk-boot releases:
 *   https://github.com/rmk-rs/rmk-boot/releases
 */

MEMORY {
  FLASH : ORIGIN = 0x10007000, LENGTH = 2031616   /* ACTIVE region, no DFU */
  RAM   : ORIGIN = 0x20000000, LENGTH = 256K                       /* SRAM */
}

/* Bootloader partition symbols — offsets relative to flash start.
 * The active/state/dfu names match embassy-boot's from_linkerfile_blocking(),
 * storage is an RMK extension.
 */
__bootloader_state_start   = 0x6000;
__bootloader_state_end     = 0x7000;
__bootloader_active_start  = 0x7000;
__bootloader_active_end    = 0x1F7000;
__bootloader_dfu_start     = 0x0;      /* unused with dfu_ext */
__bootloader_dfu_end       = 0x0;      /* unused with dfu_ext */
__bootloader_storage_start = 0x1F7000;
__bootloader_storage_end   = 0x1FF000;
