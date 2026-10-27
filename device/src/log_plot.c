// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Plots (log_plot.h): the port-60 CBOR maps of spec/protocol.md §5.6, built with ucComm's writer.

#include "log_plot.h"

#include "cbor.h"

_Static_assert(LOG_PLOT_BUF_SIZE <= LOG_MAX_PACKET_SIZE, "LOG_PLOT_BUF_SIZE");

// The map written into b (from its start) as one port-60 frame.
static bool send(const uint8_t *b, cbor_stream_t *s) {
  if (s->error != CBOR_ERROR_NONE) return false;
  size_t n = cbor_read_avail(s);
  if (log_tx_avail() < n) return false;
  log_tx(LOG_PLOT_PORT, b, n);
  return true;
}

static void texts(cbor_stream_t *s, const char *const *v, size_t n) {
  cbor_write_array(s, n);
  for (size_t i = 0; i < n; i++) cbor_write_text(s, v[i] ? v[i] : "");
}

static void floats(cbor_stream_t *s, const float *v, size_t n) {
  cbor_write_array(s, n);
  for (size_t i = 0; i < n; i++) cbor_write_float32(s, v[i]);
}

// "yser", "y2ser", "y3ser" (and the same for limits and props)
static void key(cbor_stream_t *s, size_t axis, const char *what) {
  char k[12] = {'y'};
  size_t n = 1;
  if (axis > 0) k[n++] = (char)('1' + axis);
  while (*what && n < sizeof(k) - 1) k[n++] = *what++;
  cbor_write_textn(s, k, n);
}

bool log_plot_create(const char *name, const char *xlabel, const log_plot_axis_t *axes, size_t n_axes,
                     uint32_t xscroll) {
  if (n_axes == 0 || n_axes > LOG_PLOT_AXES) return false;
  size_t entries = 3 + (xscroll ? 1 : 0); // plot, xlabel, ylabel (all axes), xscroll; per axis:
  for (size_t a = 0; a < n_axes; a++) {
    if (axes[a].n == 0 || axes[a].n > LOG_PLOT_SERIES) return false;
    entries += 1 + (axes[a].lo < axes[a].hi ? 1 : 0) + (axes[a].props ? 1 : 0);
  }
  uint8_t b[LOG_PLOT_BUF_SIZE];
  cbor_stream_t s;
  cbor_init(&s, b, sizeof(b));
  cbor_write_map(&s, entries);
  cbor_write_text(&s, "plot");
  cbor_write_text(&s, name);
  cbor_write_text(&s, "xlabel");
  cbor_write_text(&s, xlabel ? xlabel : "");
  cbor_write_text(&s, "ylabel");
  cbor_write_array(&s, n_axes);
  for (size_t a = 0; a < n_axes; a++) cbor_write_text(&s, axes[a].label ? axes[a].label : "");
  for (size_t a = 0; a < n_axes; a++) {
    key(&s, a, "ser");
    texts(&s, axes[a].series, axes[a].n);
    if (axes[a].lo < axes[a].hi) {
      key(&s, a, "limits");
      float l[2] = {axes[a].lo, axes[a].hi};
      floats(&s, l, 2);
    }
    if (axes[a].props) {
      key(&s, a, "props");
      texts(&s, axes[a].props, axes[a].n);
    }
  }
  if (xscroll) {
    cbor_write_text(&s, "xscroll");
    cbor_write_uint64(&s, xscroll);
  }
  return send(b, &s);
}

bool log_plot_data(const char *name, uint8_t axis, uint8_t series, const float *xs, const float *ys, size_t n) {
  if (axis < 1 || axis > LOG_PLOT_AXES || series < 1 || series > LOG_PLOT_SERIES) return false;
  uint8_t b[LOG_PLOT_BUF_SIZE];
  cbor_stream_t s;
  cbor_init(&s, b, sizeof(b));
  cbor_write_map(&s, xs ? 3 : 2);
  cbor_write_text(&s, "plot");
  cbor_write_text(&s, name);
  if (xs) {
    cbor_write_text(&s, "x");
    floats(&s, xs, n);
  }
  char k[3] = {'y', (char)('0' + axis), (char)('0' + series)};
  cbor_write_textn(&s, k, 3);
  floats(&s, ys, n);
  return send(b, &s);
}

bool log_plot_point(const char *name, float x, const float *ys, size_t n) {
  if (n == 0 || n > LOG_PLOT_SERIES) return false;
  uint8_t b[LOG_PLOT_BUF_SIZE];
  cbor_stream_t s;
  cbor_init(&s, b, sizeof(b));
  cbor_write_map(&s, 2 + n);
  cbor_write_text(&s, "plot");
  cbor_write_text(&s, name);
  cbor_write_text(&s, "x");
  floats(&s, &x, 1);
  for (size_t i = 0; i < n; i++) {
    char k[3] = {'y', '1', (char)('1' + i)};
    cbor_write_textn(&s, k, 3);
    floats(&s, &ys[i], 1);
  }
  return send(b, &s);
}

bool log_plot_clear(const char *name) {
  uint8_t b[LOG_PLOT_BUF_SIZE];
  cbor_stream_t s;
  cbor_init(&s, b, sizeof(b));
  cbor_write_map(&s, 1);
  cbor_write_text(&s, "clear_plot");
  cbor_write_text(&s, name);
  return send(b, &s);
}
