#!/usr/bin/env bash
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
#
# Fetch the parts of an STM32Cube package the test firmware builds against (ST's code is not in
# this repository): a shallow, sparse clone of github.com/STMicroelectronics/<repo> at <tag>.
#
#   fetch-stm32cube.sh <repo> <tag> <dest> <path>... [-- <submodule>...]
#
# e.g. fetch-stm32cube.sh STM32CubeL4 v1.18.2 cube/l4 Drivers/CMSIS/Include \
#        -- Drivers/STM32L4xx_HAL_Driver Drivers/CMSIS/Device/ST/STM32L4xx
set -euo pipefail
repo=$1 tag=$2 dest=$3
shift 3
paths=() subs=()
while (($#)); do
  if [[ $1 == -- ]]; then shift; subs=("$@"); break; fi
  paths+=("$1"); shift
done
git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$tag" --filter=blob:none --sparse \
  "https://github.com/STMicroelectronics/$repo" "$dest"
# --skip-checks: submodule paths are not directories in the index
git -C "$dest" sparse-checkout set --skip-checks "${paths[@]}" "${subs[@]}"
if ((${#subs[@]})); then
  git -C "$dest" submodule --quiet update --init --depth 1 "${subs[@]}"
fi
