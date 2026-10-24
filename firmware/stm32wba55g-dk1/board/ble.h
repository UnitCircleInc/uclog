// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// BLE peripheral for the ucLog link (protocol.md §4.2) on ST's STM32WBA BLE stack: one service
// and one characteristic, both UUID 8683b8f3-0000-1000-8000-00805f9b34fb. Device → host:
// notifications of up to ATT MTU − 3 bytes; host → device: writes (with or without response, long
// writes too) into an RX ring. Like the UART driver, TX drains a ring buffer (the log's) after an
// optional prefix.
//
// The stack and this module run in the lowest-priority interrupt (ble_irq), so the BLE link keeps
// moving while the application busy-waits.

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "cb.h"

#define BLE_IRQn TSC_IRQn // an otherwise unused vector, pended by software
#define BLE_IRQ_PRIO 15   // the lowest; also the link layer's "radio SW low" interrupt

void ble_init(const char *name);
void ble_irq(void);
void ble_tick(void); // every millisecond
bool ble_connected(void); // and the host subscribed to notifications

// RX: received bytes. The counter changes with every byte.
size_t ble_rx_avail(void);
const uint8_t *ble_rx_peek(void);
void ble_rx_skip(size_t n);
void ble_rx_discard(void);
uint32_t ble_rx_count(void);
bool ble_rx_overflow(void); // bytes were lost (ring full) since the last call

// TX: call with interrupts disabled. `tx_cb` is drained as notifications are queued.
void ble_tx_attach(cb_t *tx_cb);
void ble_tx_schedule(const uint8_t *prefix, size_t n);
void ble_tx_pause(bool pause);
bool ble_tx_idle(void);
void ble_tx_abort(void); // forget the prefix (another link takes over)
