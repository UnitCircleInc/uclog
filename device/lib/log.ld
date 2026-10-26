/*
 * © 2026 Unit Circle Inc.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * @file
 * @brief Linker command/script file
 *
 * Linker script for the Cortex-M platforms.
 */

.logdata (INFO) :
{
  . = ALIGN(4);
  _slogstr = .;
  KEEP(*(.logstr))
  KEEP(*(.logstr.*))
} > LOGDATA
