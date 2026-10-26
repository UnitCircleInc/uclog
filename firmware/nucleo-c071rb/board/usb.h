// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// USB device (STM32 USB DRD FS) for the ucLog links (protocol.md §4.1, §4.3): a composite device,
// VID 2FE3, with
//   interface 0    a placeholder vendor HID (so that the ucLog HID is interface 1),
//   interface 1    the ucLog HID: input report A1 and output report A2, 63 data bytes each,
//   interfaces 2-3 CDC-ACM (a serial port).
// Each function has its own RX ring and TX engine; the TX engine drains a ring buffer (the log's)
// in packets, after an optional prefix, like the UART driver.

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "cb.h"

typedef enum {
  USB_FN_HID = 0,
  USB_FN_CDC = 1,
} usb_fn_t;

void usb_init(const uint8_t uid[12]);
void usb_irq(void);
bool usb_configured(void);

// RX: received bytes (HID: the report ID is removed). The counter changes with every byte.
size_t usb_rx_avail(usb_fn_t fn);
const uint8_t *usb_rx_peek(usb_fn_t fn);
void usb_rx_skip(usb_fn_t fn, size_t n);
void usb_rx_discard(usb_fn_t fn);
uint32_t usb_rx_count(usb_fn_t fn);

// TX: call with interrupts disabled. `tx_cb` is drained as packets are sent.
void usb_tx_attach(usb_fn_t fn, cb_t *tx_cb);
void usb_tx_schedule(usb_fn_t fn, const uint8_t *prefix, size_t n);
void usb_tx_pause(usb_fn_t fn, bool pause);
// Every millisecond: send what is waiting even if it does not fill a packet.
void usb_tx_tick(void);
bool usb_tx_idle(usb_fn_t fn);
// Take back a packet the host never collected (it stays in the ring); the function is idle after.
void usb_tx_abort(usb_fn_t fn);
// Interrupts off: send what is buffered by polling the peripheral (gives up after `ms` without
// progress, e.g. when no host reads).
void usb_tx_flush_polled(usb_fn_t fn, uint32_t ms);
