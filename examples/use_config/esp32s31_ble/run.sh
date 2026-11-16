#!/usr/bin/env bash
set -euo pipefail

elf="$1"
shift
image="${elf}.bin"
espflash save-image --chip esp32s31 --flash-size 16mb --partition-table partitions.csv --merge --skip-padding "$elf" "$image"
exec esptool --chip esp32s31 --before no-reset --after hard-reset "$@" write-flash --flash-size 16MB 0x0 "$image"
