// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Plots: line plots shown by the hosts, sent as CBOR maps on port 60 (spec/protocol.md §5.6). A
// plot has up to 3 y axes with up to 9 series each. Data for a plot the host has not seen created
// (it connected later) is dropped there, so create plots again when a host connects (for example
// when log_is_host_ready() turns true).
//
// Each message is encoded on the stack in LOG_PLOT_BUF_SIZE bytes (at most LOG_MAX_PACKET_SIZE);
// the calls return false when it does not fit, or when log_tx() has no room (the frame is dropped).

#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "log.h"

#ifdef __cplusplus
extern "C" {
#endif

#if !defined(LOG_PLOT_BUF_SIZE)
#define LOG_PLOT_BUF_SIZE 256
#endif

#define LOG_PLOT_PORT 60
#define LOG_PLOT_AXES 3
#define LOG_PLOT_SERIES 9

typedef struct {
  const char *label;          // the axis label
  const char *const *series;  // the series names (1 to LOG_PLOT_SERIES)
  size_t n;
  float lo, hi;               // the axis range; lo >= hi: automatic
  const char *const *props;   // NULL, or n renderer hints (e.g. "infinite": a line without x)
} log_plot_axis_t;

// Create the line plot `name` (or clear it if the host has it). `xscroll`: points kept per series
// (0: all of them).
bool log_plot_create(const char *name, const char *xlabel, const log_plot_axis_t *axes, size_t n_axes,
                     uint32_t xscroll);

// Append n points to series `series` (1-9) of axis `axis` (1-3). xs NULL: x continues from the
// series' last x (or the points have no x, for an "infinite" series).
bool log_plot_data(const char *name, uint8_t axis, uint8_t series, const float *xs, const float *ys, size_t n);

// Append the point (x, ys[i]) to series i + 1 of axis 1, for i < n.
bool log_plot_point(const char *name, float x, const float *ys, size_t n);

// Clear the plot's data.
bool log_plot_clear(const char *name);

#ifdef __cplusplus
}
#endif
