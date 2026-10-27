// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0

#pragma once

#include "log.h"

extern uint8_t host_tx[];
extern size_t host_tx_n;
extern uint32_t host_ms;

void host_rx_feed(const uint8_t *b, size_t n);
