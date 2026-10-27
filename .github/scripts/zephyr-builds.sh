#!/usr/bin/env bash
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# The device CI's Zephyr builds (.github/workflows/device.yml), with ZEPHYR_BASE set to a workspace
# from .github/zephyr/v*/west.yml. The ucLog repository is the module (zephyr/module.yml at its
# root), with its own copy of ucComm.
#
#   zephyr-builds.sh <out dir>
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd)
out=$1
z=$repo/.github/zephyr
major=$(sed -n 's/^VERSION_MAJOR = //p' "$ZEPHYR_BASE/VERSION")
failed=0

build() { # name board app [cmake args...]
  local name=$1 board=$2 app=$3
  shift 3
  if west build -p always -b "$board" -d "$out/$name" "$app" -- \
      -DZEPHYR_EXTRA_MODULES="$repo" "$@" > "$out/$name.log" 2>&1; then
    echo "ok     $name: $(grep -E '^ +FLASH:' "$out/$name.log" | awk '{print $2, $3}')"
  else
    echo "FAILED $name"; tail -30 "$out/$name.log"; failed=1
  fi
}

mkdir -p "$out"
build example-l452-ucuart_z nucleo_l452re/stm32l452xx/p "$repo/device/examples/zephyr"
build example-nrf-ucuart nrf52840dk/nrf52840 "$repo/device/examples/zephyr"
build example-nrf-ucusb nrf52840dk/nrf52840 "$repo/device/examples/zephyr" -DEXTRA_DTC_OVERLAY_FILE="$z/ucusb.overlay"
build example-nrf-ucuart_z nrf52840dk/nrf52840 "$repo/device/examples/zephyr" -DEXTRA_DTC_OVERLAY_FILE="$z/ucuart_z-nrf.overlay"
build example-nrf-options nrf52840dk/nrf52840 "$repo/device/examples/zephyr" -DEXTRA_CONF_FILE="$z/options.conf" \
  -DSNIPPET="uclog-target1;uclog-shell"
if ((major >= 4)); then # the test firmware needs Zephyr 4's POSIX options
  build test-firmware-l452 nucleo_l452re/stm32l452xx/p "$repo/firmware/zephyr"
fi
exit $failed
