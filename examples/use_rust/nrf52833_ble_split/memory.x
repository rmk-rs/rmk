MEMORY
{
  /* NOTE 1 K = 1 KiB = 1024 bytes */
  /* nRF52833 (512K flash, 128K RAM) WITH the Adafruit nRF52 bootloader, which
     owns 0x74000 upwards. That leaves 460K for the app plus its storage, and a
     vial + USB + BLE split central already needs about 426K of it, so FLASH
     here is deliberately almost everything that is left. */
  FLASH   : ORIGIN = 0x00001000, LENGTH = 444K
  /* 4 x 4K pages, mirrored by `StorageConfig` in src/central.rs and
     src/peripheral.rs. Keep the two in sync. */
  STORAGE : ORIGIN = 0x00070000, LENGTH = 16K
  RAM     : ORIGIN = 0x20000008, LENGTH = 127K

  /* Without a bootloader, use the whole chip instead: */
  /* FLASH : ORIGIN = 0x00000000, LENGTH = 496K */
  /* STORAGE : ORIGIN = 0x0007C000, LENGTH = 16K */
  /* RAM : ORIGIN = 0x20000000, LENGTH = 128K */
}
