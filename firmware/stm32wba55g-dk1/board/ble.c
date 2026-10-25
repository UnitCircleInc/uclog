// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// BLE peripheral for the ucLog link on the STM32WBA55 (ble.h). Built on ST's BLE stack the way
// its BLE_p2pServer_Simplest example uses it: no sequencer, no timer server, no low power. The
// link layer and the host stack ask for background processing through ll_process() and
// Ble_HostStack_Process(); here both pend ble_irq(), which runs them, then handles the stack's
// events (queued by BLECB_Indication: no stack call may be made from inside BleStack_Process),
// then sends notifications.

#include "ble.h"

#include <string.h>

#include <stm32wbaxx.h>

#include "app_common.h" // app_conf.h, DIVC, MAX
#include "ble_core.h"
#include "blestack.h"
#include "ll_sys.h"
#include "skel_ble.h"

#include "board.h"
#include "log.h"

// 8683b8f3-0000-1000-8000-00805f9b34fb, least significant byte first (service and characteristic).
static const uint8_t uclog_uuid[16] = {0xfb, 0x34, 0x9b, 0x5f, 0x80, 0x00, 0x00, 0x80,
                                       0x00, 0x10, 0x00, 0x00, 0xf3, 0xb8, 0x83, 0x86};

#define CHAR_MAX_LEN 512 // the longest write the characteristic takes (a long write)
#define NOTIFY_MAX (CFG_BLE_ATT_MTU_MAX - 3)

// Host stack buffers (sizes from the example's app_conf.h, except the TX/RX packet pool). The pool
// is counted in 32-byte blocks; the example's margin (21) holds about two full notifications. 512
// blocks (16 KiB) let ~60 full notifications wait in the stack, so every connection event can be
// filled.
#undef CFG_BLE_MBLOCK_COUNT_MARGIN
#define CFG_BLE_MBLOCK_COUNT_MARGIN 512
#define GATT_BUF_SIZE                                                                              \
  BLE_TOTAL_BUFFER_SIZE_GATT(CFG_BLE_NUM_GATT_ATTRIBUTES, CFG_BLE_NUM_GATT_SERVICES,             \
                             CFG_BLE_ATT_VALUE_ARRAY_SIZE)
#define DYN_ALLOC_SIZE BLE_TOTAL_BUFFER_SIZE(CFG_BLE_NUM_LINK, CFG_BLE_MBLOCK_COUNT, 0)
#define NVM_SIZE 4 // no bonding: a dummy NVM cache
static uint32_t host_buffer[(DYN_ALLOC_SIZE + 3) / 4];
static uint32_t gatt_buffer[(GATT_BUF_SIZE + 3) / 4];
static uint64_t host_nvm_buffer[NVM_SIZE];

// Events from the stack, waiting to be handled: [uint16 length][HCI event packet] records in a ring.
static uint8_t evq_buf[4096];
static cb_t evq = CB_INIT(evq_buf);

static uint8_t rx_buf[2048];
static cb_t rx_cb = CB_INIT(rx_buf);

static volatile uint32_t work; // WORK_* bits
#define WORK_LL (1u << 0)
#define WORK_HOST (1u << 1)

static struct {
  uint16_t svc, chr;        // handles
  uint16_t conn;            // connection handle, valid while connected
  bool connected, subscribed;
  uint16_t mtu;             // ATT MTU of the connection
  uint16_t ll_tx;           // link layer payload (data length), 27 until extended
  uint32_t connected_ms;    // when (for the connection interval request)
  bool conn_update_sent;
  bool pool_wait;           // stack out of TX buffers: wait for ACI_GATT_TX_POOL_AVAILABLE
  bool pumping;             // a notification is being built from the ring
  bool flush;               // the producer paused: a short notification may go
  size_t last_avail;        // bytes in the ring at the last tick
  cb_t *tx_cb;
  const uint8_t *prefix;
  size_t prefix_n;
  bool paused;
  volatile uint32_t rx_total;
  volatile bool rx_overflow;
  uint32_t evq_dropped;
  uint8_t scan_rsp[31];
  uint8_t scan_rsp_n;
} b;

static void pend(void) {
  NVIC_SetPendingIRQ(BLE_IRQn);
}

// ---------------------------------------------------------------------------------------------
// Stack callbacks (host_stack_if.c, ll_sys_if.c, the stack)

void ll_process(void) {
  __atomic_fetch_or(&work, WORK_LL, __ATOMIC_RELAXED);
  pend();
}

void Ble_HostStack_Process(void) {
  __atomic_fetch_or(&work, WORK_HOST, __ATOMIC_RELAXED);
  pend();
}

// From BleStack_Process(): queue HCI events for ble_irq(). An event is at most 258 bytes (`ext_data`
// is not used for events: an attribute value too long for one event comes in several).
uint8_t BLECB_Indication(const uint8_t *data, uint16_t length, const uint8_t *ext_data, uint16_t ext_length) {
  (void)ext_data;
  (void)ext_length;
  if (length < 3 || data[0] != HCI_EVENT_PKT_TYPE) return BLE_STATUS_SUCCESS; // no ACL data here
  uint16_t n = (uint16_t)(3u + data[2]);
  if (n > length) n = length;
  if (cb_write_avail(&evq) < n + 2u) {
    b.evq_dropped++;
    return BLE_STATUS_SUCCESS;
  }
  cb_write(&evq, &n, 2);
  cb_write(&evq, data, n);
  return BLE_STATUS_SUCCESS;
}

// ---------------------------------------------------------------------------------------------
// GAP / GATT

static void check(tBleStatus s, const char *what) {
  if (s != BLE_STATUS_SUCCESS) LOG_ERROR("ble: %s failed: 0x%02x", what, s);
}

static void advertise(void) {
  check(hci_le_set_scan_response_data(b.scan_rsp_n, b.scan_rsp), "scan response");
  uint8_t uuids[17] = {AD_TYPE_128_BIT_SERV_UUID_CMPLT_LIST};
  memcpy(uuids + 1, uclog_uuid, 16);
  // 30-50 ms advertising interval (0.625 ms units); no name in the advertisement (the 128-bit
  // UUID fills it): the name is in the scan response.
  check(aci_gap_set_discoverable(ADV_IND, 48, 80, GAP_STATIC_RANDOM_ADDR, NO_WHITE_LIST_USE, 0, NULL,
                                 sizeof(uuids), uuids, 0, 0),
        "advertising");
}

void ble_init(const char *name) {
  NVIC_SetPriority(BLE_IRQn, BLE_IRQ_PRIO);
  NVIC_EnableIRQ(BLE_IRQn);

  static BleStack_init_t p;
  p.numAttrRecord = CFG_BLE_NUM_GATT_ATTRIBUTES;
  p.numAttrServ = CFG_BLE_NUM_GATT_SERVICES;
  p.attrValueArrSize = CFG_BLE_ATT_VALUE_ARRAY_SIZE;
  p.prWriteListSize = CFG_BLE_ATTR_PREPARE_WRITE_VALUE_SIZE;
  p.attMtu = CFG_BLE_ATT_MTU_MAX;
  p.max_coc_nbr = CFG_BLE_COC_NBR_MAX;
  p.max_coc_mps = CFG_BLE_COC_MPS_MAX;
  p.max_coc_initiator_nbr = CFG_BLE_COC_INITIATOR_NBR_MAX;
  p.numOfLinks = CFG_BLE_NUM_LINK;
  p.mblockCount = CFG_BLE_MBLOCK_COUNT;
  p.bleStartRamAddress = (uint8_t *)host_buffer;
  p.total_buffer_size = DYN_ALLOC_SIZE;
  p.bleStartRamAddress_GATT = (uint8_t *)gatt_buffer;
  p.total_buffer_size_GATT = GATT_BUF_SIZE;
  p.nvm_cache_buffer = host_nvm_buffer;
  p.nvm_cache_max_size = NVM_SIZE;
  p.nvm_cache_size = NVM_SIZE - 1;
  p.options = CFG_BLE_OPTIONS;
  p.debug = 0;

  bool en = board_lock(); // ble_irq() must not run the stack meanwhile
  tBleStatus s = BleStack_Init(&p);
  if (s != BLE_STATUS_SUCCESS) {
    board_unlock(en);
    LOG_ERROR("ble: stack init failed: 0x%02x", s);
    return;
  }

  // Static random address from the chip's unique ID (the two top bits set).
  const uint8_t *uid = board_uid();
  uint8_t addr[6];
  for (int i = 0; i < 6; i++) addr[i] = uid[i] ^ uid[i + 6];
  addr[5] |= 0xc0;
  check(aci_hal_write_config_data(CONFIG_DATA_RANDOM_ADDRESS_OFFSET, sizeof(addr), addr), "address");
  check(aci_gatt_init(), "GATT init");
  uint16_t gap_svc, gap_name, gap_appearance;
  size_t name_n = strlen(name);
  if (name_n > sizeof(b.scan_rsp) - 2) name_n = sizeof(b.scan_rsp) - 2;
  check(aci_gap_init(GAP_PERIPHERAL_ROLE, PRIVACY_DISABLED, (uint8_t)name_n, &gap_svc, &gap_name, &gap_appearance),
        "GAP init");
  check(aci_gatt_update_char_value(gap_svc, gap_name, 0, (uint8_t)name_n, (const uint8_t *)name), "name");

  // Longest packets and the 2M PHY where the host agrees.
  check(hci_le_write_suggested_default_data_length(251, 2120), "data length");
  check(hci_le_set_default_phy(0, HCI_TX_PHYS_LE_2M_PREF | 1, HCI_RX_PHYS_LE_2M_PREF | 1), "PHY");

  Service_UUID_t su;
  memcpy(su.Service_UUID_128, uclog_uuid, 16);
  check(aci_gatt_add_service(UUID_TYPE_128, &su, PRIMARY_SERVICE, 4, &b.svc), "add service");
  Char_UUID_t cu;
  memcpy(cu.Char_UUID_128, uclog_uuid, 16);
  check(aci_gatt_add_char(b.svc, UUID_TYPE_128, &cu, CHAR_MAX_LEN,
                          CHAR_PROP_WRITE | CHAR_PROP_WRITE_WITHOUT_RESP | CHAR_PROP_NOTIFY, ATTR_PERMISSION_NONE,
                          GATT_NOTIFY_ATTRIBUTE_WRITE, 16, 1, &b.chr),
        "add characteristic");

  b.scan_rsp[0] = (uint8_t)(name_n + 1);
  b.scan_rsp[1] = AD_TYPE_COMPLETE_LOCAL_NAME;
  memcpy(b.scan_rsp + 2, name, name_n);
  b.scan_rsp_n = (uint8_t)(name_n + 2);
  advertise();
  board_unlock(en);
  pend();
  LOG_INFO("ble: advertising as \"%s\", address %02x:%02x:%02x:%02x:%02x:%02x", name, addr[5], addr[4], addr[3],
           addr[2], addr[1], addr[0]);
}

// ---------------------------------------------------------------------------------------------
// Events

static uint16_t u16(const uint8_t *p) {
  return (uint16_t)(p[0] | (p[1] << 8));
}

static void rx_put(const uint8_t *d, size_t n) {
  bool en = board_lock();
  size_t room = cb_write_avail(&rx_cb);
  if (n > room) {
    b.rx_overflow = true;
    n = room;
  }
  cb_write(&rx_cb, d, n);
  b.rx_total += n;
  board_unlock(en);
}

static void on_connected(uint16_t conn, uint16_t interval) {
  b.conn = conn;
  b.connected = true;
  b.subscribed = false;
  b.mtu = 23;
  b.ll_tx = 27;
  b.pool_wait = false;
  b.connected_ms = board_ms();
  b.conn_update_sent = false;
  LOG_INFO("ble: connected (handle 0x%04x), interval %u.%02u ms", conn, interval * 125 / 100, interval * 125 % 100);
  // Longest link layer packets (251 bytes) and the 2M PHY: the host may not ask for them itself.
  check(hci_le_set_data_length(conn, 251, 2120), "data length request");
  check(hci_le_set_phy(conn, 0, HCI_TX_PHYS_LE_2M_PREF, HCI_RX_PHYS_LE_2M_PREF, 0), "PHY request");
}

static void on_event(const uint8_t *e, size_t n) {
  if (n < 3) return;
  const uint8_t *p = e + 3; // parameters
  size_t pn = n - 3;
  switch (e[1]) {
  case HCI_DISCONNECTION_COMPLETE_EVT_CODE:
    // Status, Connection_Handle, Reason
    if (pn >= 4) LOG_INFO("ble: disconnected (reason 0x%02x)", p[3]);
    b.connected = false;
    b.subscribed = false;
    b.pool_wait = false;
    advertise();
    break;
  case HCI_LE_META_EVT_CODE:
    // Subevent, then (mostly) Status, Connection_Handle
    if (pn >= 14 && p[0] == HCI_LE_CONNECTION_COMPLETE_SUBEVT_CODE) {
      if (p[1] == 0) on_connected(u16(p + 2), u16(p + 12));
    } else if (pn >= 26 && (p[0] == HCI_LE_ENHANCED_CONNECTION_COMPLETE_SUBEVT_CODE ||
                            p[0] == HCI_LE_ENHANCED_CONNECTION_COMPLETE_V2_SUBEVT_CODE)) {
      if (p[1] == 0) on_connected(u16(p + 2), u16(p + 24)); // after the two private addresses
    } else if (pn >= 10 && p[0] == HCI_LE_CONNECTION_UPDATE_COMPLETE_SUBEVT_CODE) {
      uint16_t i = u16(p + 4);
      LOG_INFO("ble: connection interval %u.%02u ms, latency %u, status 0x%02x", i * 125 / 100, i * 125 % 100, u16(p + 6),
               p[1]);
    } else if (pn >= 11 && p[0] == HCI_LE_DATA_LENGTH_CHANGE_SUBEVT_CODE) {
      // Connection_Handle, MaxTxOctets, MaxTxTime, MaxRxOctets, MaxRxTime
      b.ll_tx = u16(p + 3);
      LOG_INFO("ble: data length tx %u bytes / %u us, rx %u bytes / %u us", u16(p + 3), u16(p + 5), u16(p + 7),
               u16(p + 9));
    } else if (pn >= 6 && p[0] == HCI_LE_PHY_UPDATE_COMPLETE_SUBEVT_CODE) {
      LOG_INFO("ble: PHY tx %uM, rx %uM, status 0x%02x", p[4], p[5], p[1]);
    }
    break;
  case HCI_VENDOR_SPECIFIC_DEBUG_EVT_CODE: {
    if (pn < 2) break;
    uint16_t code = u16(p);
    const uint8_t *d = p + 2;
    size_t dn = pn - 2;
    if (code == ACI_GATT_ATTRIBUTE_MODIFIED_VSEVT_CODE && dn >= 8) {
      // Connection_Handle, Attr_Handle, Offset (bit 15: more of a long write to come),
      // Attr_Data_Length, Attr_Data
      uint16_t attr = u16(d + 2), len = u16(d + 6);
      if (len > dn - 8) len = (uint16_t)(dn - 8);
      if (attr == b.chr + 1) {
        rx_put(d + 8, len);
      } else if (attr == b.chr + 2 && len >= 1) {
        b.subscribed = (d[8] & 1) != 0; // CCCD
        LOG_INFO("ble: notifications %s", b.subscribed ? "on" : "off");
      }
    } else if (code == ACI_ATT_EXCHANGE_MTU_RESP_VSEVT_CODE && dn >= 4) {
      b.mtu = u16(d + 2);
      LOG_INFO("ble: ATT MTU %u", b.mtu);
    } else if (code == ACI_L2CAP_CONNECTION_UPDATE_RESP_VSEVT_CODE && dn >= 4) {
      LOG_INFO("ble: connection parameters %s by the host", u16(d + 2) == 0 ? "accepted" : "rejected");
    } else if (code == ACI_GATT_TX_POOL_AVAILABLE_VSEVT_CODE) {
      b.pool_wait = false;
    }
    break;
  }
  default:
    break;
  }
}

// ---------------------------------------------------------------------------------------------
// TX

static size_t cb_copy(const cb_t *cb, uint8_t *dst, size_t max) {
  size_t n = cb_read_avail(cb);
  if (n > max) n = max;
  size_t first = cb_peek_avail(cb);
  if (first > n) first = n;
  memcpy(dst, cb_peek(cb), first);
  memcpy(dst + first, cb->b, n - first);
  return n;
}

// Queue notifications until the ring is empty or the stack has no buffers left. Only full ones
// (ATT MTU - 3) while the log keeps coming: each notification costs a link layer packet whatever
// its size. A short one goes once the producer has paused (ble_tick) or for the device info.
static void tx_pump(void) {
  static uint8_t buf[NOTIFY_MAX];
  while (b.connected && b.subscribed && !b.pool_wait) {
    bool en = board_lock();
    // ATT MTU - 3, but with the data length extended, no more than one link layer packet holds
    // (data length - 4 L2CAP - 3 ATT header bytes: 244 of 251): a 248-byte notification would take
    // a second packet for its last 4 bytes. Without the extension, the MTU's worth (fragmented)
    // has the fewest headers.
    size_t room = b.mtu - 3u;
    if (b.ll_tx > 27 && room > b.ll_tx - 7u) room = b.ll_tx - 7u;
    if (room > sizeof(buf)) room = sizeof(buf);
    const uint8_t *prefix = b.prefix;
    size_t np = b.prefix_n < room ? b.prefix_n : room;
    memcpy(buf, prefix, np);
    size_t n = np;
    if (!b.paused && b.tx_cb) n += cb_copy(b.tx_cb, buf + np, room - np);
    bool go = n == room || (n > 0 && (b.flush || np > 0));
    b.pumping = go;
    board_unlock(en);
    if (!go) return;
    tBleStatus s = aci_gatt_update_char_value_ext(b.conn, b.svc, b.chr, 0x01, (uint16_t)n, 0, (uint8_t)n, buf);
    en = board_lock();
    b.pumping = false;
    if (s == BLE_STATUS_SUCCESS) {
      if (n < room) b.flush = false;
      if (b.prefix == prefix && b.prefix_n >= np) { // not replaced or aborted (ble_tx_abort) meanwhile
        b.prefix += np;
        b.prefix_n -= np;
      }
      if (n > np) cb_skip(b.tx_cb, n - np);
    }
    board_unlock(en);
    if (s == BLE_STATUS_INSUFFICIENT_RESOURCES) {
      b.pool_wait = true;
    } else if (s != BLE_STATUS_SUCCESS) {
      LOG_ERROR("ble: notification failed: 0x%02x", s);
      return;
    }
  }
}

// ---------------------------------------------------------------------------------------------
// The BLE interrupt

void ble_irq(void) {
  uint32_t w = __atomic_exchange_n(&work, 0, __ATOMIC_RELAXED);
  if (w & WORK_LL) ll_sys_bg_process();
  if (w & WORK_HOST) {
    if (BleStack_Process() == BLE_SLEEPMODE_RUNNING) Ble_HostStack_Process(); // more to do
  }
  static uint8_t e[258];
  while (cb_read_avail(&evq) > 0) {
    uint16_t n;
    cb_read(&evq, &n, 2);
    cb_read(&evq, e, n);
    on_event(e, n);
  }
  if (b.evq_dropped) {
    LOG_ERROR("ble: %lu stack events dropped", (unsigned long)b.evq_dropped);
    b.evq_dropped = 0;
  }
  // Ask for a 15 ms connection interval, no latency, 2 s supervision timeout (within Apple's
  // accessory guidelines) once the link has settled: the request is refused (0x0C) while the
  // connection's first procedures run. Asking for 7.5-15 ms makes no difference with macOS.
  if (b.connected && !b.conn_update_sent && board_ms() - b.connected_ms > 1000) {
    b.conn_update_sent = true;
    check(aci_l2cap_connection_parameter_update_req(b.conn, 12, 12, 0, 200), "connection parameters");
  }
  tx_pump();
}

// Every millisecond (SysTick): if the ring has not grown since the last tick, the producer has
// paused, so what is there goes now even if it does not fill a notification.
void ble_tick(void) {
  if (!b.connected || !b.subscribed || b.tx_cb == NULL) return;
  size_t n = cb_read_avail(b.tx_cb);
  if (n > 0 && n == b.last_avail && !b.flush) {
    b.flush = true;
    pend();
  }
  b.last_avail = n;
  if (!b.conn_update_sent && board_ms() - b.connected_ms > 1000) pend();
}

bool ble_connected(void) {
  return b.connected && b.subscribed;
}

// ---------------------------------------------------------------------------------------------
// RX and TX interface (ble.h)

size_t ble_rx_avail(void) {
  return cb_peek_avail(&rx_cb);
}

const uint8_t *ble_rx_peek(void) {
  return cb_peek(&rx_cb);
}

void ble_rx_skip(size_t n) {
  // uclog-clientv2: never past the data (a link handover or reset may have emptied the ring
  // since the caller's peek).
  size_t a = cb_read_avail(&rx_cb);
  cb_skip(&rx_cb, n < a ? n : a);
}

void ble_rx_discard(void) {
  while (ble_rx_avail() > 0) ble_rx_skip(ble_rx_avail());
}

uint32_t ble_rx_count(void) {
  return b.rx_total;
}

bool ble_rx_overflow(void) {
  // Read and clear together: an overflow flagged in between must not be lost.
  bool en = board_lock();
  bool o = b.rx_overflow;
  b.rx_overflow = false;
  board_unlock(en);
  return o;
}

void ble_tx_attach(cb_t *tx_cb) {
  b.tx_cb = tx_cb;
}

void ble_tx_schedule(const uint8_t *prefix, size_t n) {
  if (prefix != NULL) {
    b.prefix = prefix;
    b.prefix_n = n;
  }
  pend();
}

void ble_tx_pause(bool pause) {
  b.paused = pause;
  if (!pause) pend();
}

bool ble_tx_idle(void) {
  return !b.pumping && b.prefix_n == 0;
}

void ble_tx_abort(void) {
  b.prefix_n = 0;
}

// Link layer debug signals (GPIO toggles in ST's examples): unused.
void LINKLAYER_DEBUG_SIGNAL_SET(void *signal) {
  (void)signal;
}

void LINKLAYER_DEBUG_SIGNAL_RESET(void *signal) {
  (void)signal;
}

void LINKLAYER_DEBUG_SIGNAL_TOGGLE(void *signal) {
  (void)signal;
}
