// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// A small USB device stack for the STM32 USB DRD FS peripheral (STM32C0, G0, H5, U5): control
// endpoint, descriptors, and the two ucLog functions (usb.h). Full speed, single-buffered
// endpoints; the packet memory (PMA) is accessed in 32-bit words.
//
// Endpoints (each CHEP register serves one IN and one OUT endpoint of the same type):
//   0  control            64
//   1  IN  interrupt  8   placeholder HID (interface 0, never sends)
//   2  IN/OUT interrupt 64 ucLog HID (interface 1)
//   3  IN  interrupt  8   CDC notifications (interface 2, never sends)
//   4  IN/OUT bulk    64  CDC data (interface 3)

#include "usb.h"

#include <string.h>

#include <stm32c0xx.h>

#include "log.h"

#define VID 0x2FE3 // as Zephyr's samples; the ucLog hosts look for it (protocol.md §4.1, §4.3)
#define PID 0x0100
#define EP0_SIZE 64
#define PKT 64
#define HID_IN_ID 0xA1
#define HID_OUT_ID 0xA2
#define RX_RING 512

// ---------------------------------------------------------------------------------------------
// Descriptors

#define LO(x) ((uint8_t)((x) & 0xFF))
#define HI(x) ((uint8_t)((x) >> 8))

static const uint8_t device_desc[18] = {
    18, 1, 0x00, 0x02, // USB 2.0
    0xEF, 0x02, 0x01,  // miscellaneous: interface association descriptors
    EP0_SIZE, LO(VID), HI(VID), LO(PID), HI(PID), 0x00, 0x01, // bcdDevice 1.00
    1, 2, 3,           // manufacturer, product, serial number strings
    1,                 // configurations
};

// The placeholder (interface 0): a vendor page with one input byte that is never sent.
static const uint8_t hid0_report[] = {
    0x06, 0x02, 0xFF, // usage page (vendor 0xFF02)
    0x09, 0x01,       // usage 1
    0xA1, 0x01,       // collection (application)
    0x15, 0x00, 0x26, 0xFF, 0x00, 0x75, 0x08, 0x95, 0x01, // 0..255, 8 bits, 1 byte
    0x09, 0x01, 0x81, 0x02, // input (data, variable, absolute)
    0xC0,
};

// The ucLog HID (interface 1, protocol.md §4.3): a vendor page (WebHID refuses others), input
// report A1 and output report A2 with 63 bytes each.
static const uint8_t hid1_report[] = {
    0x06, 0x00, 0xFF, // usage page (vendor 0xFF00)
    0x09, 0x01,       // usage 1
    0xA1, 0x01,       // collection (application)
    0x15, 0x00, 0x26, 0xFF, 0x00, 0x75, 0x08, // 0..255, 8 bits
    0x85, HID_IN_ID, 0x95, 63, 0x09, 0x02, 0x81, 0x02,  // report A1: 63 bytes, input
    0x85, HID_OUT_ID, 0x95, 63, 0x09, 0x03, 0x91, 0x02, // report A2: 63 bytes, output
    0xC0,
};

#define CONFIG_LEN (9 + (9 + 9 + 7) + (9 + 9 + 7 + 7) + 8 + (9 + 5 + 5 + 4 + 5 + 7) + (9 + 7 + 7))

static const uint8_t config_desc[CONFIG_LEN] = {
    9, 2, LO(CONFIG_LEN), HI(CONFIG_LEN), 4, 1, 0, 0x80, 50, // 4 interfaces, bus powered, 100 mA

    // Interface 0: placeholder HID
    9, 4, 0, 0, 1, 0x03, 0x00, 0x00, 6,
    9, 0x21, 0x11, 0x01, 0, 1, 0x22, LO(sizeof(hid0_report)), HI(sizeof(hid0_report)),
    7, 5, 0x81, 0x03, 8, 0, 255,

    // Interface 1: ucLog HID
    9, 4, 1, 0, 2, 0x03, 0x00, 0x00, 4,
    9, 0x21, 0x11, 0x01, 0, 1, 0x22, LO(sizeof(hid1_report)), HI(sizeof(hid1_report)),
    7, 5, 0x82, 0x03, PKT, 0, 1, // IN, interrupt, every frame
    7, 5, 0x02, 0x03, PKT, 0, 1, // OUT

    // Interfaces 2-3: CDC-ACM
    8, 0x0B, 2, 2, 0x02, 0x02, 0x00, 5, // interface association
    9, 4, 2, 0, 1, 0x02, 0x02, 0x00, 5, // communication: ACM
    5, 0x24, 0x00, 0x10, 0x01,          // header 1.10
    5, 0x24, 0x01, 0x00, 3,             // call management: data interface 3
    4, 0x24, 0x02, 0x02,                // ACM: line coding and serial state
    5, 0x24, 0x06, 2, 3,                // union: 2 controls 3
    7, 5, 0x83, 0x03, 8, 0, 16,         // notifications (never sent)
    9, 4, 3, 0, 2, 0x0A, 0x00, 0x00, 0, // data
    7, 5, 0x04, 0x02, PKT, 0,  0,       // OUT, bulk
    7, 5, 0x84, 0x02, PKT, 0,  0,       // IN, bulk
};

static const char *const strings[] = {
    [1] = "Unit Circle",
    [2] = "ucLog test NUCLEO-C071RB",
    [4] = "ucLog HID",
    [5] = "ucLog serial",
    [6] = "reserved",
};
static char serial[25]; // the chip's unique ID in hex

// ---------------------------------------------------------------------------------------------
// Peripheral access

#define EPR(n) (*(&USB_DRD_FS->CHEP0R + (n)))
#define BD(n) (USB_DRD_PMA_BUFF[n])

// PMA layout (byte offsets; the buffer descriptor table is fixed at 0).
enum {
  PMA_EP0_TX = 0x40,
  PMA_EP0_RX = 0x80,
  PMA_EP1_TX = 0xC0,
  PMA_EP2_TX = 0x100,
  PMA_EP2_RX = 0x140,
  PMA_EP3_TX = 0x180,
  PMA_EP4_TX = 0x1C0,
  PMA_EP4_RX = 0x200,
};
#define RX_64 ((1u << 31) | (1u << 26)) // BLSIZE = 32 bytes, NUM_BLOCK = 2 blocks

static void pma_write(uint32_t off, const uint8_t *src, size_t n) {
  volatile uint32_t *p = (volatile uint32_t *)(USB_DRD_PMAADDR + off);
  for (size_t i = 0; i < n; i += 4) {
    uint32_t w = 0;
    for (size_t k = 0; k < 4 && i + k < n; k++) w |= (uint32_t)src[i + k] << (8 * k);
    *p++ = w;
  }
}

// A packet already in a word-aligned buffer: one 32-bit write per word (~40% fewer cycles than
// assembling words from bytes, which the TX interrupt did for every packet).
static void pma_write_words(uint32_t off, const uint32_t *src, size_t n) {
  volatile uint32_t *p = (volatile uint32_t *)(USB_DRD_PMAADDR + off);
  for (size_t i = 0; i < (n + 3) / 4; i++) p[i] = src[i];
}

static void pma_read(uint32_t off, uint8_t *dst, size_t n) {
  volatile uint32_t *p = (volatile uint32_t *)(USB_DRD_PMAADDR + off);
  for (size_t i = 0; i < n; i += 4) {
    uint32_t w = *p++;
    for (size_t k = 0; k < 4 && i + k < n; k++) dst[i + k] = (uint8_t)(w >> (8 * k));
  }
}

static void tx_set(uint8_t ep, uint32_t pma, size_t n) {
  BD(ep).TXBD = ((uint32_t)n << 16) | pma;
}

static size_t rx_count(uint8_t ep) {
  return (BD(ep).RXBD >> 16) & 0x3FF;
}

// The STAT and DTOG bits toggle when written with 1; VTRX/VTTX clear when written with 0.
static void ep_stat_tx(uint8_t ep, uint32_t stat) {
  uint32_t r = EPR(ep) & USB_CHEP_TX_DTOGMASK;
  EPR(ep) = (r ^ stat) | USB_CHEP_VTRX | USB_CHEP_VTTX;
}

static void ep_stat_rx(uint8_t ep, uint32_t stat) {
  uint32_t r = EPR(ep) & USB_CHEP_RX_DTOGMASK;
  EPR(ep) = (r ^ stat) | USB_CHEP_VTRX | USB_CHEP_VTTX;
}

static void ep_clear_vtrx(uint8_t ep) {
  EPR(ep) = (EPR(ep) & USB_CHEP_REG_MASK & ~USB_CHEP_VTRX) | USB_CHEP_VTTX;
}

static void ep_clear_vttx(uint8_t ep) {
  EPR(ep) = (EPR(ep) & USB_CHEP_REG_MASK & ~USB_CHEP_VTTX) | USB_CHEP_VTRX;
}

// One direction's data toggle back to DATA0 (CLEAR_FEATURE(ENDPOINT_HALT), SET_INTERFACE).
static void ep_dtog_clear(uint8_t ep, bool in) {
  uint32_t r = EPR(ep), bit = in ? USB_CHEP_DTOG_TX : USB_CHEP_DTOG_RX;
  EPR(ep) = (r & USB_CHEP_REG_MASK) | (r & bit) | USB_CHEP_VTRX | USB_CHEP_VTTX;
}

// Endpoint type and address; both data toggles back to DATA0; both directions disabled.
static void ep_open(uint8_t ep, uint32_t type) {
  uint32_t r = EPR(ep);
  EPR(ep) = (r & USB_CHEP_REG_MASK & ~(USB_CHEP_UTYPE | USB_CHEP_ADDR | USB_CHEP_KIND)) | type | ep |
            USB_CHEP_VTRX | USB_CHEP_VTTX;
  r = EPR(ep);
  EPR(ep) = (r & (USB_CHEP_REG_MASK | USB_CHEP_DTOG_RX | USB_CHEP_DTOG_TX | USB_CHEP_TX_STTX | USB_CHEP_RX_STRX)) |
            USB_CHEP_VTRX | USB_CHEP_VTTX; // writing back the 1s clears toggles and stats
}

// ---------------------------------------------------------------------------------------------
// Functions: RX rings and TX engines

typedef struct {
  uint8_t ep;
  uint32_t tx_pma, rx_pma;
  bool hid;
  // RX
  uint8_t ring[RX_RING];
  cb_t rx;
  volatile uint32_t rx_count;
  volatile bool rx_held; // a packet waits in the PMA for room in the ring
  // TX
  cb_t *tx_cb;
  const uint8_t *prefix;
  size_t prefix_n, prefix_off;
  volatile size_t inflight; // bytes in the armed IN packet (0: idle)
  bool inflight_prefix;
  volatile bool zlp_inflight; // a zero-length packet ends a bulk transfer of full packets
  bool need_zlp;
  bool paused;
  bool flush; // a packet shorter than the maximum may go (set every millisecond by usb_tx_tick())
} fn_t;

static fn_t fns[2] = {
    [USB_FN_HID] = {.ep = 2, .tx_pma = PMA_EP2_TX, .rx_pma = PMA_EP2_RX, .hid = true},
    [USB_FN_CDC] = {.ep = 4, .tx_pma = PMA_EP4_TX, .rx_pma = PMA_EP4_RX},
};

static volatile bool configured;
static uint8_t halted_in, halted_out; // bit n: endpoint n is halted (SET_FEATURE(ENDPOINT_HALT))
static uint8_t line_coding[7] = {0x40, 0x42, 0x0F, 0x00, 0, 0, 8}; // 1000000 8N1

static void fn_reset(fn_t *f) {
  cb_reset(&f->rx);
  f->rx_held = false;
  f->inflight = 0;
  f->zlp_inflight = f->need_zlp = false;
  f->prefix_n = f->prefix_off = 0;
}

// Copy a received OUT packet into the ring, or leave it in the PMA (NAK) until there is room.
static void fn_rx_take(fn_t *f) {
  size_t n = rx_count(f->ep);
  uint8_t b[PKT];
  size_t skip = 0;
  if (n > PKT) n = PKT;
  pma_read(f->rx_pma, b, n);
  if (f->hid) {
    // Report A2 (the ID byte first); anything else is not ours.
    if (n == 0 || b[0] != HID_OUT_ID) n = 0;
    else skip = 1;
  }
  if (cb_write_avail(&f->rx) < n - skip) {
    f->rx_held = true; // the endpoint stays NAK until usb_rx_skip() makes room
    return;
  }
  if (n > skip) {
    cb_write(&f->rx, b + skip, n - skip);
    f->rx_count += n - skip;
  }
  f->rx_held = false;
  if (!(halted_out & (1u << f->ep))) ep_stat_rx(f->ep, USB_EP_RX_VALID);
}

// Up to `max` bytes from the start of the ring, across its wrap, without consuming them. A packet
// must not end early at the wrap: the HID's zero padding would split a frame (00 delimits).
static size_t cb_copy(const cb_t *cb, uint8_t *dst, size_t max) {
  size_t n = cb_read_avail(cb);
  if (n > max) n = max;
  size_t first = cb_peek_avail(cb);
  if (first > n) first = n;
  memcpy(dst, cb_peek(cb), first);
  memcpy(dst + first, cb->b, n - first);
  return n;
}

static void fn_tx_next(fn_t *f) {
  if (f->inflight || f->zlp_inflight || f->paused || !configured || f->tx_cb == NULL) return;
  if (halted_in & (1u << f->ep)) return;
  uint32_t w[PKT / 4];
  uint8_t *b = (uint8_t *)w;
  uint8_t *data = f->hid ? b + 1 : b;
  size_t room = f->hid ? PKT - 1 : PKT, n;
  if (f->prefix_off < f->prefix_n) {
    n = f->prefix_n - f->prefix_off;
    if (n > room) n = room;
    memcpy(data, f->prefix + f->prefix_off, n);
    f->inflight_prefix = true;
  } else {
    // Wait for a full packet, or until the next millisecond tick: a burst of small logs would
    // otherwise go out one log per packet (and per interrupt) as fast as the CPU makes them.
    if (!f->flush && cb_read_avail(f->tx_cb) < room) return;
    n = cb_copy(f->tx_cb, data, room);
    f->inflight_prefix = false;
  }
  if (n == 0) {
    // CDC: a transfer that ended with a full packet is not complete for the host until a short
    // one arrives.
    if (f->need_zlp) {
      f->need_zlp = false;
      f->zlp_inflight = true;
      tx_set(f->ep, f->tx_pma, 0);
      ep_stat_tx(f->ep, USB_EP_TX_VALID);
    }
    return;
  }
  if (f->hid) {
    // Report A1, zero padded after whole frames (00 is a frame delimiter, protocol.md §4.3).
    b[0] = HID_IN_ID;
    memset(b + 1 + n, 0, PKT - 1 - n);
    pma_write_words(f->tx_pma, w, PKT);
    tx_set(f->ep, f->tx_pma, PKT);
  } else {
    pma_write_words(f->tx_pma, w, n);
    tx_set(f->ep, f->tx_pma, n);
  }
  f->inflight = n;
  f->flush = false;
  f->need_zlp = !f->hid && n == PKT;
  ep_stat_tx(f->ep, USB_EP_TX_VALID);
}

static void fn_tx_done(fn_t *f) {
  if (f->zlp_inflight) {
    f->zlp_inflight = false;
    fn_tx_next(f);
    return;
  }
  size_t n = f->inflight;
  if (n == 0) return;
  if (f->inflight_prefix) {
    f->prefix_off += n;
    if (f->prefix_off >= f->prefix_n) f->prefix_n = f->prefix_off = 0;
  } else {
    cb_skip(f->tx_cb, n);
  }
  f->inflight = 0;
  fn_tx_next(f);
}

// ---------------------------------------------------------------------------------------------
// Control endpoint

typedef struct __attribute__((packed)) {
  uint8_t type, req;
  uint16_t value, index, length;
} setup_t;

static struct {
  const uint8_t *data;
  size_t left;
  bool zlp;
  uint8_t addr;   // set after the status stage
  setup_t out;    // a request waiting for its OUT data stage
  bool out_wait;
  uint8_t buf[64];
} ctl;

static void ctl_stall(void) {
  ep_stat_tx(0, USB_EP_TX_STALL);
  ep_stat_rx(0, USB_EP_RX_STALL);
}

static void ctl_send_next(void) {
  size_t n = ctl.left > EP0_SIZE ? EP0_SIZE : ctl.left;
  pma_write(PMA_EP0_TX, ctl.data, n);
  tx_set(0, PMA_EP0_TX, n);
  ctl.data += n;
  ctl.left -= n;
  ep_stat_tx(0, USB_EP_TX_VALID);
}

static void ctl_send(const uint8_t *d, size_t n, uint16_t want) {
  if (n > want) n = want;
  ctl.data = d;
  ctl.left = n;
  ctl.zlp = n < want && n % EP0_SIZE == 0; // a short transfer ends with a short packet
  ctl_send_next();
}

static void ctl_status_in(void) {
  ctl.data = NULL;
  ctl.left = 0;
  ctl.zlp = false;
  tx_set(0, PMA_EP0_TX, 0);
  ep_stat_tx(0, USB_EP_TX_VALID);
}

static size_t string_desc(uint8_t i, uint8_t *b) {
  if (i == 0) {
    b[0] = 4, b[1] = 3, b[2] = 0x09, b[3] = 0x04; // English (US)
    return 4;
  }
  const char *s = i == 3 ? serial : (i < sizeof(strings) / sizeof(strings[0]) ? strings[i] : NULL);
  if (s == NULL) return 0;
  size_t n = strlen(s);
  if (n > 31) n = 31;
  b[0] = (uint8_t)(2 + 2 * n), b[1] = 3;
  for (size_t k = 0; k < n; k++) b[2 + 2 * k] = (uint8_t)s[k], b[3 + 2 * k] = 0;
  return 2 + 2 * n;
}

static fn_t *fn_for(uint8_t ep) {
  for (int i = 0; i < 2; i++) {
    if (fns[i].ep == ep) return &fns[i];
  }
  return NULL;
}

// Endpoint addresses in the configuration descriptor (0x81-0x84 IN, 0x02 and 0x04 OUT).
static bool ep_exists(uint8_t a) {
  uint8_t ep = a & 0x7F;
  return (a & 0x80) ? ep >= 1 && ep <= 4 : ep == 2 || ep == 4;
}

// SET_FEATURE / CLEAR_FEATURE(ENDPOINT_HALT). Clearing a halt (even one not set) resets the data
// toggle to DATA0, as the host does; an IN packet armed before the halt is sent after it.
static void ep_halt(uint8_t a, bool halt) {
  uint8_t ep = a & 0x7F, bit = (uint8_t)(1u << ep);
  fn_t *f = fn_for(ep);
  if (a & 0x80) {
    bool was = halted_in & bit;
    if (halt) {
      halted_in |= bit;
      ep_stat_tx(ep, USB_EP_TX_STALL);
      return;
    }
    halted_in &= (uint8_t)~bit;
    ep_dtog_clear(ep, true);
    if (was) ep_stat_tx(ep, f && (f->inflight || f->zlp_inflight) ? USB_EP_TX_VALID : USB_EP_TX_NAK);
    if (f) fn_tx_next(f);
  } else {
    bool was = halted_out & bit;
    if (halt) {
      halted_out |= bit;
      ep_stat_rx(ep, USB_EP_RX_STALL);
      return;
    }
    halted_out &= (uint8_t)~bit;
    ep_dtog_clear(ep, false);
    if (was) ep_stat_rx(ep, f && f->rx_held ? USB_EP_RX_NAK : USB_EP_RX_VALID);
  }
}

static void configure(void) {
  halted_in = halted_out = 0;
  ep_open(1, USB_EP_INTERRUPT);
  tx_set(1, PMA_EP1_TX, 0);
  ep_stat_tx(1, USB_EP_TX_NAK);
  ep_open(3, USB_EP_INTERRUPT);
  tx_set(3, PMA_EP3_TX, 0);
  ep_stat_tx(3, USB_EP_TX_NAK);
  ep_open(2, USB_EP_INTERRUPT);
  ep_open(4, USB_EP_BULK);
  for (int i = 0; i < 2; i++) {
    fn_t *f = &fns[i];
    fn_reset(f);
    BD(f->ep).RXBD = RX_64 | f->rx_pma;
    tx_set(f->ep, f->tx_pma, 0);
    ep_stat_tx(f->ep, USB_EP_TX_NAK);
    ep_stat_rx(f->ep, USB_EP_RX_VALID);
  }
  configured = true;
  for (int i = 0; i < 2; i++) fn_tx_next(&fns[i]);
}

static void on_setup(const setup_t *s) {
  uint8_t recipient = s->type & 0x1F, kind = (s->type >> 5) & 3;
  uint8_t iface = (uint8_t)s->index;
  static uint8_t two[2];
  if (kind == 0) { // standard
    switch (s->req) {
    case 6: { // GET_DESCRIPTOR
      uint8_t t = HI(s->value), i = LO(s->value);
      if (recipient == 1) { // interface: HID class descriptors
        if (t == 0x22 && iface == 0) return ctl_send(hid0_report, sizeof(hid0_report), s->length);
        if (t == 0x22 && iface == 1) return ctl_send(hid1_report, sizeof(hid1_report), s->length);
        if (t == 0x21 && iface <= 1) return ctl_send(config_desc + (iface == 0 ? 18 : 43), 9, s->length);
        return ctl_stall();
      }
      if (t == 1) return ctl_send(device_desc, sizeof(device_desc), s->length);
      if (t == 2) return ctl_send(config_desc, sizeof(config_desc), s->length);
      if (t == 3) {
        size_t n = string_desc(i, ctl.buf);
        if (n == 0) return ctl_stall();
        return ctl_send(ctl.buf, n, s->length);
      }
      return ctl_stall(); // device qualifier (full speed only), others
    }
    case 5: // SET_ADDRESS
      ctl.addr = (uint8_t)(s->value & 0x7F);
      return ctl_status_in();
    case 9: // SET_CONFIGURATION
      if (s->value == 1) configure();
      else configured = false;
      return ctl_status_in();
    case 8: // GET_CONFIGURATION
      two[0] = configured;
      return ctl_send(two, 1, s->length);
    case 0: // GET_STATUS: an endpoint's halt bit
      two[0] = two[1] = 0;
      if (recipient == 2) {
        uint8_t a = (uint8_t)s->index, bit = (uint8_t)(1u << (a & 0x7F));
        if ((a & 0x7F) != 0 && !ep_exists(a)) return ctl_stall();
        two[0] = ((a & 0x80) ? halted_in : halted_out) & bit ? 1 : 0;
      }
      return ctl_send(two, 2, s->length);
    case 10: // GET_INTERFACE
      two[0] = 0;
      return ctl_send(two, 1, s->length);
    case 1: // CLEAR_FEATURE
    case 3: // SET_FEATURE
      // ENDPOINT_HALT only (no remote wakeup); endpoint 0's halt is a no-op.
      if (recipient != 2 || s->value != 0) return ctl_stall();
      if ((s->index & 0x7F) == 0) return ctl_status_in();
      if (!configured || !ep_exists((uint8_t)s->index)) return ctl_stall();
      ep_halt((uint8_t)s->index, s->req == 3);
      return ctl_status_in();
    case 11: { // SET_INTERFACE: alternate setting 0 only; its endpoints back to DATA0, not halted
      static const uint8_t eps[4][2] = {{0x81}, {0x82, 0x02}, {0x83}, {0x84, 0x04}};
      if (!configured || iface > 3 || s->value != 0) return ctl_stall();
      for (int i = 0; i < 2 && eps[iface][i]; i++) ep_halt(eps[iface][i], false);
      return ctl_status_in();
    }
    }
    return ctl_stall();
  }
  if (kind == 1 && recipient == 1) { // class, interface
    if (iface <= 1) { // HID
      switch (s->req) {
      case 0x01: // GET_REPORT: the fallback to interrupt reads; an empty report
        memset(ctl.buf, 0, sizeof(ctl.buf));
        ctl.buf[0] = iface == 1 ? HID_IN_ID : 0;
        return ctl_send(ctl.buf, iface == 1 ? 64 : 1, s->length);
      case 0x09: // SET_REPORT: an output report over the control pipe
        if (s->length == 0 || s->length > 64) return ctl_stall();
        ctl.out = *s;
        ctl.out_wait = true;
        return ep_stat_rx(0, USB_EP_RX_VALID);
      case 0x0A: // SET_IDLE
      case 0x0B: // SET_PROTOCOL
        return ctl_status_in();
      }
      return ctl_stall();
    }
    switch (s->req) { // CDC
    case 0x20: // SET_LINE_CODING
      if (s->length != 7) return ctl_stall();
      ctl.out = *s;
      ctl.out_wait = true;
      return ep_stat_rx(0, USB_EP_RX_VALID);
    case 0x21: // GET_LINE_CODING
      return ctl_send(line_coding, sizeof(line_coding), s->length);
    case 0x22: // SET_CONTROL_LINE_STATE
    case 0x23: // SEND_BREAK
      return ctl_status_in();
    }
  }
  ctl_stall();
}

static void on_ctl_out_data(void) {
  size_t n = rx_count(0);
  if (n > sizeof(ctl.buf)) n = sizeof(ctl.buf);
  pma_read(PMA_EP0_RX, ctl.buf, n);
  ctl.out_wait = false;
  if (ctl.out.req == 0x20) {
    if (n == 7) memcpy(line_coding, ctl.buf, 7);
  } else if (ctl.out.req == 0x09 && LO(ctl.out.index) == 1) {
    fn_t *f = &fns[USB_FN_HID];
    if (n > 1 && ctl.buf[0] == HID_OUT_ID && cb_write_avail(&f->rx) >= n - 1) {
      cb_write(&f->rx, ctl.buf + 1, n - 1);
      f->rx_count += n - 1;
    }
  }
  ctl_status_in();
}

static void on_ep0(void) {
  uint32_t r = EPR(0);
  if (r & USB_CHEP_VTRX) {
    bool setup = (r & USB_CHEP_SETUP) != 0;
    if (setup) {
      uint8_t b[8];
      pma_read(PMA_EP0_RX, b, 8);
      ep_clear_vtrx(0);
      setup_t s;
      memcpy(&s, b, sizeof(s));
      ctl.out_wait = false;
      on_setup(&s);
    } else {
      ep_clear_vtrx(0);
      if (ctl.out_wait) on_ctl_out_data();
      else ep_stat_rx(0, USB_EP_RX_VALID); // status stage of an IN transfer: ready for the next
    }
  }
  if (EPR(0) & USB_CHEP_VTTX) {
    ep_clear_vttx(0);
    if (ctl.addr) {
      USB_DRD_FS->DADDR = USB_DADDR_EF | ctl.addr;
      ctl.addr = 0;
    }
    if (ctl.left > 0) {
      ctl_send_next();
    } else if (ctl.zlp) {
      ctl.zlp = false;
      tx_set(0, PMA_EP0_TX, 0);
      ep_stat_tx(0, USB_EP_TX_VALID);
    } else if (ctl.data != NULL) {
      ctl.data = NULL;
      ep_stat_rx(0, USB_EP_RX_VALID); // the host's status OUT
    }
  }
}

// ---------------------------------------------------------------------------------------------
// Bus events

static void bus_reset(void) {
  configured = false;
  ep_open(0, USB_EP_CONTROL);
  tx_set(0, PMA_EP0_TX, 0);
  BD(0).RXBD = RX_64 | PMA_EP0_RX;
  ep_stat_rx(0, USB_EP_RX_VALID);
  ep_stat_tx(0, USB_EP_TX_NAK);
  memset(&ctl, 0, sizeof(ctl));
  halted_in = halted_out = 0;
  for (int i = 0; i < 2; i++) fn_reset(&fns[i]);
  USB_DRD_FS->DADDR = USB_DADDR_EF;
}

void usb_irq(void) {
  uint32_t istr = USB_DRD_FS->ISTR;
  if (istr & USB_ISTR_RESET) {
    USB_DRD_FS->ISTR = ~USB_ISTR_RESET;
    bus_reset();
    return;
  }
  while ((istr = USB_DRD_FS->ISTR) & USB_ISTR_CTR) {
    uint8_t ep = istr & USB_ISTR_IDN;
    if (ep == 0) {
      on_ep0();
      continue;
    }
    uint32_t r = EPR(ep);
    fn_t *f = ep == fns[USB_FN_HID].ep ? &fns[USB_FN_HID] : ep == fns[USB_FN_CDC].ep ? &fns[USB_FN_CDC] : NULL;
    if (r & USB_CHEP_VTRX) {
      ep_clear_vtrx(ep);
      if (f) fn_rx_take(f);
    }
    if (r & USB_CHEP_VTTX) {
      ep_clear_vttx(ep);
      if (f) fn_tx_done(f);
    }
  }
  if (istr & (USB_ISTR_SUSP | USB_ISTR_WKUP | USB_ISTR_ERR | USB_ISTR_PMAOVR | USB_ISTR_SOF | USB_ISTR_ESOF)) {
    USB_DRD_FS->ISTR = ~(USB_ISTR_SUSP | USB_ISTR_WKUP | USB_ISTR_ERR | USB_ISTR_PMAOVR | USB_ISTR_SOF | USB_ISTR_ESOF);
  }
}

void usb_init(const uint8_t uid[12]) {
  static const char hex[] = "0123456789ABCDEF";
  for (int i = 0; i < 12; i++) {
    serial[2 * i] = hex[uid[11 - i] >> 4];
    serial[2 * i + 1] = hex[uid[11 - i] & 15];
  }
  for (int i = 0; i < 2; i++) cb_init(&fns[i].rx, fns[i].ring, sizeof(fns[i].ring));

  // 48 MHz from HSIUSB48, trimmed to the host's start-of-frame by the CRS.
  RCC->CR |= RCC_CR_HSIUSB48ON;
  while (!(RCC->CR & RCC_CR_HSIUSB48RDY)) {
  }
  RCC->CCIPR2 &= ~RCC_CCIPR2_USBSEL; // HSIUSB48
  RCC->APBENR1 |= RCC_APBENR1_USBEN | RCC_APBENR1_CRSEN;
  CRS->CFGR = (CRS->CFGR & ~CRS_CFGR_SYNCSRC) | CRS_CFGR_SYNCSRC_1; // USB SOF
  CRS->CR |= CRS_CR_AUTOTRIMEN | CRS_CR_CEN;

  USB_DRD_FS->CNTR = USB_CNTR_USBRST; // powered, held in reset
  for (volatile int i = 0; i < 100; i++) {
  }
  USB_DRD_FS->CNTR = 0;
  USB_DRD_FS->ISTR = 0;
  USB_DRD_FS->CNTR = USB_CNTR_CTRM | USB_CNTR_RESETM;
  NVIC_SetPriority(USB_DRD_FS_IRQn, 1);
  NVIC_EnableIRQ(USB_DRD_FS_IRQn);
  USB_DRD_FS->BCDR |= USB_BCDR_DPPU; // attach
  LOG_INFO("usb: %04x:%04x serial %s", VID, PID, serial);
}

bool usb_configured(void) {
  return configured;
}

// ---------------------------------------------------------------------------------------------
// Function API

size_t usb_rx_avail(usb_fn_t fn) {
  return cb_peek_avail(&fns[fn].rx);
}

const uint8_t *usb_rx_peek(usb_fn_t fn) {
  return cb_peek(&fns[fn].rx);
}

void usb_rx_skip(usb_fn_t fn, size_t n) {
  fn_t *f = &fns[fn];
  NVIC_DisableIRQ(USB_DRD_FS_IRQn);
  // Never past the data: a bus reset may have emptied the ring since the caller's peek.
  size_t a = cb_read_avail(&f->rx);
  cb_skip(&f->rx, n < a ? n : a);
  if (f->rx_held && cb_write_avail(&f->rx) >= PKT) fn_rx_take(f);
  NVIC_EnableIRQ(USB_DRD_FS_IRQn);
}

void usb_rx_discard(usb_fn_t fn) {
  usb_rx_skip(fn, cb_read_avail(&fns[fn].rx));
}

uint32_t usb_rx_count(usb_fn_t fn) {
  return fns[fn].rx_count;
}

void usb_tx_attach(usb_fn_t fn, cb_t *tx_cb) {
  fns[fn].tx_cb = tx_cb;
}

void usb_tx_schedule(usb_fn_t fn, const uint8_t *prefix, size_t n) {
  fn_t *f = &fns[fn];
  if (prefix != NULL) {
    f->prefix = prefix;
    f->prefix_n = n;
    f->prefix_off = 0;
  }
  fn_tx_next(f);
}

void usb_tx_tick(void) {
  for (int i = 0; i < 2; i++) {
    fns[i].flush = true;
    fn_tx_next(&fns[i]);
  }
}

void usb_tx_pause(usb_fn_t fn, bool pause) {
  fns[fn].paused = pause;
}

bool usb_tx_idle(usb_fn_t fn) {
  return fns[fn].inflight == 0 && !fns[fn].zlp_inflight;
}

void usb_tx_abort(usb_fn_t fn) {
  fn_t *f = &fns[fn];
  if (f->inflight == 0 && !f->zlp_inflight) return;
  bool paused = f->paused;
  f->paused = true; // fn_tx_done() must not arm the next packet
  if (!(halted_in & (1u << f->ep))) ep_stat_tx(f->ep, USB_EP_TX_NAK);
  if (EPR(f->ep) & USB_CHEP_VTTX) { // it went out after all
    ep_clear_vttx(f->ep);
    fn_tx_done(f);
  }
  f->inflight = 0;
  f->zlp_inflight = f->need_zlp = false;
  f->paused = paused;
}

void usb_tx_flush_polled(usb_fn_t fn, uint32_t ms) {
  fn_t *f = &fns[fn];
  // ~ms milliseconds without progress at 48 MHz (about 12 cycles per loop).
  uint32_t idle = 0, limit = ms * 4000;
  size_t last = (size_t)-1;
  f->paused = false;
  f->flush = true;
  fn_tx_next(f);
  while (configured && (f->inflight || cb_peek_avail(f->tx_cb) || f->prefix_n)) {
    usb_irq();
    f->flush = true;
    fn_tx_next(f);
    size_t left = cb_read_avail(f->tx_cb) + f->prefix_n - f->prefix_off;
    if (left != last) {
      last = left;
      idle = 0;
    } else if (++idle > limit) {
      break;
    }
  }
}
