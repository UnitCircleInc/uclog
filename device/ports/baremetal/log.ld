/*
 * © 2026 Unit Circle Inc.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Log format strings. INFO = not loaded into the target; the addresses only identify the
 * strings. The LOGDATA region sets the target digit: (address >> 20) & 0xF.
 * Include inside SECTIONS {}.
 */

.logdata (INFO) :
{
  . = ALIGN(4);
  _slogstr = .;
  KEEP(*(.logstr))
  KEEP(*(.logstr.*))
} > LOGDATA
