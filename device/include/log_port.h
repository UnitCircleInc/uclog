// © 2025 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// The port (link) interface: implemented by the board on bare metal, by ports/zephyr on Zephyr;
// called by log.c. TX drains `tx_cb` (e.g. by DMA); RX is a ring buffer read with
// avail/peek/skip. Host presence (spec/protocol.md §2.3) is log.c's: the port only pauses TX
// when told and reports that bytes arrived (rx_mark).

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "cb.h"

#ifdef __cplusplus
extern "C" {
#endif

uint32_t log_port_lock(void);                          // disable interrupts, return previous state
void log_port_unlock(uint32_t key);                    // restore
uint32_t log_port_ms(void);                            // millisecond counter
void log_port_init(cb_t *tx_cb);                       // start the link; TX takes data from tx_cb
void log_port_tx_schedule(const uint8_t *prefix, size_t n); // start TX: prefix (if any), then tx_cb
void log_port_tx_pause(bool pause);                    // stop starting new TX transfers
bool log_port_tx_idle(void);                           // no TX transfer in progress
void log_port_panic(void);                             // switch to polled TX (interrupts off)
size_t log_port_rx_avail(void);                        // contiguous received bytes
const uint8_t *log_port_rx_peek(void);
void log_port_rx_skip(size_t n);
uint32_t log_port_rx_mark(void);                       // changes whenever a byte is received
uint32_t log_port_rx_check(void);                      // restart RX after UART errors; nonzero if any
__attribute__((noreturn)) void log_port_fatal(void);   // breakpoint if debugging, else reset

// Threads (optional: log.c has weak defaults for a single-threaded port, where log_poll() runs in
// the main loop and log_rx() runs log_poll() while it waits).
bool log_port_is_poller(void);  // this thread runs log_poll() (default: true)
void log_port_yield(void);      // log_poll() from another thread: let the poller run (default: nothing)
void log_port_rx_begin(void);   // log_rx() callers take turns (default: nothing)
void log_port_rx_end(void);
void log_port_rx_wait(void);    // wait a while for log_port_rx_wake() (default: log_poll())
void log_port_rx_wake(void);    // a log_rx() reply arrived, or the host went away (default: nothing)

#ifdef __cplusplus
}
#endif
