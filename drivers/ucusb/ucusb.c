// © 2023 Unit Circle Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#include <stdint.h>
#include <string.h>
#include <nrfx_power.h>
#include <nrf_usbd_common.h>
#include <log.h>

#include "cb.h"
#include "cobs.h"
#include "cbor.h"

#include "ucuart.h"

// Turn off logging - only used during development
#undef LOG_INFO
#undef LOG_MEM_INFO
#undef LOG_WARN
#undef LOG_ERROR
#define LOG_INFO(...)
#define LOG_MEM_INFO(...)
#define LOG_WARN(...)
#define LOG_ERROR(...)

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/irq.h>
#include <zephyr/drivers/hwinfo.h>

#include <zephyr/drivers/clock_control.h>
#include <zephyr/drivers/clock_control/nrf_clock_control.h>

#define DT_DRV_COMPAT unitcircle_ucusb

#define APP_HASH_SIZE 64
#ifdef CONFIG_SIGNED_IMAGE
#include "lib/uc/sbl.h"
#else
#if CONFIG_UC_LOG_SAVE_ENABLED
extern const uint8_t app_hash__[APP_HASH_SIZE];
#else
const __attribute__((section(".apphash"))) uint8_t app_hash__[APP_HASH_SIZE];
#endif
#endif

#define DEVICE_INFO_UCLOG_PORT (62)

#define MAX_LOG_TX_SIZE (256u)
static uint8_t device_info_tx_buf[COBS_ENC_SIZE(MAX_LOG_TX_SIZE)+2];
static size_t device_info_len;

// Need to be packed
typedef struct __attribute__ ((packed)) {
  uint8_t  bLength;
  uint8_t  bDescriptorType;
  uint16_t bcdUSB;
  uint8_t  bDeviceClass;
  uint8_t  bDeviceSubClass;
  uint8_t  bDeviceProtocol;
  uint8_t  bMaxPacketSize0;
  uint16_t idVendor;
  uint16_t idProduct;
  uint16_t bcdDevice;
  uint8_t  iManufacturer;
  uint8_t  iProduct;
  uint8_t  iSerialNumber;
  uint8_t  bNumConfigurations;
} usb_device_desc_t;

typedef struct __attribute__ ((packed)) {
  uint8_t  bLength;
  uint8_t  bDescriptorType;
  uint16_t bcdUSB;
  uint8_t  bDeviceClass;
  uint8_t  bDeviceSubClass;
  uint8_t  bDeviceProtocol;
  uint8_t  bMaxPacketSize0;
  uint16_t idVendor;
  uint16_t idProduct;
  uint16_t bcdDevice;
  uint8_t  iManufacturer;
  uint8_t  iProduct;
  uint8_t  iSerialNumber;
  uint8_t  bNumConfigurations;
} usb_device_qual_desc_t;

typedef struct  __attribute__ ((packed)) {
  uint8_t  bLength;
  uint8_t  bDescriptorType;
  uint16_t wTotalLength;
  uint8_t  bNumInterfaces;
  uint8_t  bConfigurationValue;
  uint8_t  iConfiguration;
  uint8_t  bmAttributes;
  uint8_t  bMaxPower;
} usb_configuration_desc_t;

typedef struct  __attribute__ ((packed)) {
  uint8_t bLength;
  uint8_t bDescriptorType;
  uint8_t bInterfaceNumber;
  uint8_t bAlternateSetting;
  uint8_t bNumEndpoints;
  uint8_t bInterfaceClass;
  uint8_t bInterfaceSubClass;
  uint8_t bInterfaceProtocol;
  uint8_t iInterface;
} usb_interface_desc_t;

typedef struct  __attribute__ ((packed)) {
  uint8_t bLength;
  uint8_t bDescriptorType;
  uint8_t bFirstInterface;
  uint8_t bInterfaceCount;
  uint8_t bFunctionClass;
  uint8_t bFunctionSubClass;
  uint8_t bFunctionProtocol;
  uint8_t iFunction;
} usb_interface_assoc_desc_t;

typedef struct  __attribute__ ((packed)) {
  uint8_t  bLength;
  uint8_t  bDescriptorType;
  uint8_t  bEndpointAddress;
  uint8_t  bmAttributes;
  uint16_t wMaxPacketSize;
  uint8_t  bInterval;
} usb_endpoint_desc_t;

typedef struct  __attribute__ ((packed)) {
  uint8_t  bLength;
  uint8_t  bDescriptorType;
  uint8_t  bDescriptorSubtype;
  uint16_t bcdCDC;
} usb_cdc_header_desc_t;

typedef struct  __attribute__ ((packed)) {
  uint8_t  bLength;
  uint8_t  bDescriptorType;
  uint8_t  bDescriptorSubtype;
  uint8_t  bmCapabilities;
  uint8_t  bDataInterface;
} usb_cdc_cm_desc_t;

typedef struct  __attribute__ ((packed)) {
  uint8_t  bLength;
  uint8_t  bDescriptorType;
  uint8_t  bDescriptorSubtype;
  uint8_t  bMasterInterface;
  uint8_t  bSlaveInterface0;
} usb_cdc_union_desc_t;

typedef struct  __attribute__ ((packed)) {
  uint8_t  bLength;
  uint8_t  bDescriptorType;
  uint8_t  bDescriptorSubtype;
  uint8_t  bmCapabilities;
} usb_cdc_acm_desc_t;

typedef struct  __attribute__ ((packed)) {
  uint8_t  bLength;
  uint8_t  bDescriptorType;
  uint16_t unicode_string[];
} usb_string_desc_t;


// USB Standard Request Codes defined in spec. Table 9-4
enum setup_req_e {
  USB_SREQ_GET_STATUS           = 0x00,
  USB_SREQ_CLEAR_FEATURE        = 0x01,
  USB_SREQ_SET_FEATURE          = 0x03,
  USB_SREQ_SET_ADDRESS          = 0x05,
  USB_SREQ_GET_DESCRIPTOR       = 0x06,
  USB_SREQ_SET_DESCRIPTOR       = 0x07,
  USB_SREQ_GET_CONFIGURATION    = 0x08,
  USB_SREQ_SET_CONFIGURATION    = 0x09,
  USB_SREQ_GET_INTERFACE        = 0x0A,
  USB_SREQ_SET_INTERFACE        = 0x0B,
  USB_SREQ_SYNCH_FRAME          = 0x0C,
};

// Descriptor Types defined in spec. Table 9-5
enum descriptor_type_e {
  USB_DESC_DEVICE               = 1,
  USB_DESC_CONFIGURATION        = 2,
  USB_DESC_STRING               = 3,
  USB_DESC_INTERFACE            = 4,
  USB_DESC_ENDPOINT             = 5,
  USB_DESC_DEVICE_QUALIFIER     = 6,
  USB_DESC_OTHER_SPEED          = 7,
  USB_DESC_INTERFACE_POWER      = 8,
  USB_DESC_INTERFACE_ASSOC      = 0xb,
  USB_DESC_CS_INTERFACE         = 0x24,
};

enum descriptor_subtype_e {
  USB_DESC_CS_INTERFACE_HEADER  = 0,
  USB_DESC_CS_INTERFACE_CM      = 1,
  USB_DESC_CS_INTERFACE_ACM     = 2,
  USB_DESC_CS_INTERFACE_UNION   = 6,
};

#define CS_SET_LINE_CODING (0x20)
#define CS_GET_LINE_CODING (0x21)
#define CS_SET_CONTROL_LINE_STATE (0x22)

#define ACM_SUBCLASS                    (0x02)


// Declare all the needed descriptors so that can easily send to host
enum usb_strings_e {
  LANGUAGES,
  MANUFACTURER,
  PRODUCT,
  SERIAL_NUMBER,
};

static const usb_string_desc_t languages = {
  .bLength = 2 + 1 * 2,
  .bDescriptorType = USB_DESC_STRING,
  .unicode_string = { 0x0409 }
};

static const usb_string_desc_t manufacturer = {
  .bLength = 2 + 11 * 2,
  .bDescriptorType = USB_DESC_STRING,
  .unicode_string = { 'U','n','i','t',' ','C','i','r','c','l','e' }
};

static const usb_string_desc_t product = {
  .bLength = 2 + 20 * 2,
  .bDescriptorType = USB_DESC_STRING,
  .unicode_string = {
    'U','n','i','t',' ','C','i','r','c','l','e',' ', '-',' ',
    'L','o','g','g','e','r' }
};

// This one in ram - so we can set serial #
static usb_string_desc_t serial_number = {
  .bLength = 2 + 16 * 2,
  .bDescriptorType = USB_DESC_STRING,
  .unicode_string = { '0','1','2','3','4','5','6','7','8','9','a','b','c','d','e','f' }
};

static const usb_string_desc_t* usb_strings[] = {
  [LANGUAGES]     = &languages,
  [MANUFACTURER]  = &manufacturer,
  [PRODUCT]       = &product,
  [SERIAL_NUMBER] = &serial_number,
};

static const usb_device_desc_t device = {
  .bLength                =  sizeof(usb_device_desc_t),
  .bDescriptorType        =    USB_DESC_DEVICE,
  .bcdUSB                 =  0x200, // USB 2.0
  .bDeviceClass           =    0x0, // Specified at interface
  .bDeviceSubClass        =    0x0,
  .bDeviceProtocol        =    0x0,
  .bMaxPacketSize0        =   NRF_USBD_COMMON_EPSIZE,
  .idVendor               = 0x2fe3,
  .idProduct              = 0x0100,
  .bcdDevice              =  0x302, // Device 3.02
  .iManufacturer          =    MANUFACTURER,
  .iProduct               =    PRODUCT,
  .iSerialNumber          =    SERIAL_NUMBER,
  .bNumConfigurations     =    0x1,
};

static const usb_device_qual_desc_t device_qual = {
  .bLength                =  sizeof(usb_device_desc_t),
  .bDescriptorType        =    USB_DESC_DEVICE,
  .bcdUSB                 =  0x200, // USB 2.0
  .bDeviceClass           =    0x0, // Specified at interface
  .bDeviceSubClass        =    0x0,
  .bDeviceProtocol        =    0x0,
  .bMaxPacketSize0        =   NRF_USBD_COMMON_EPSIZE,
  .bNumConfigurations     =    0x1,
};

static const struct  __attribute__ ((packed)) {
  usb_configuration_desc_t   config;
    usb_interface_assoc_desc_t interface_assoc;
    usb_interface_desc_t       interface0;
      usb_cdc_header_desc_t      cdc_header;
      usb_cdc_cm_desc_t          cdc_cm;
      usb_cdc_acm_desc_t         cdc_acm;
      usb_cdc_union_desc_t       cdc_union;
      usb_endpoint_desc_t        intf0_ep0;
    usb_interface_desc_t       interface1;
      usb_endpoint_desc_t        intf1_ep0;
      usb_endpoint_desc_t        intf1_ep1;
} configuration = {
  .config = {
    .bLength              = sizeof(usb_configuration_desc_t),
    .bDescriptorType      =    0x2, // Configuration
    .wTotalLength         =   sizeof(configuration), // (75 bytes)
    .bNumInterfaces       =    0x2,
    .bConfigurationValue  =    0x1,
    .iConfiguration       =    0x0,
    .bmAttributes         =   0xe0, // Self Powered, Remote Wakeup
    .bMaxPower            =   0x32, // (100 mA)
  },
  .interface_assoc = {
    .bLength           = sizeof(usb_interface_assoc_desc_t),
    .bDescriptorType   = USB_DESC_INTERFACE_ASSOC,
    .bFirstInterface   = 0,
    .bInterfaceCount   = 2,
    .bFunctionClass    = 2, // Communication and CDC Control
    .bFunctionSubClass = 2,
    .bFunctionProtocol = 0,
    .iFunction         = 0,
  },
  .interface0 = {
    .bLength            =    sizeof(usb_interface_desc_t),
    .bDescriptorType    =    USB_DESC_INTERFACE,
    .bInterfaceNumber   =    0x0,
    .bAlternateSetting  =    0x0,
    .bNumEndpoints      =    0x1,
    .bInterfaceClass    =    0x2, // CDC Communication
    .bInterfaceSubClass =    ACM_SUBCLASS,
    .bInterfaceProtocol =    0x0, // ??
    .iInterface         =    0x0,
  },
    .cdc_header = {
      .bLength            = sizeof(usb_cdc_header_desc_t),
      .bDescriptorType    = USB_DESC_CS_INTERFACE,
      .bDescriptorSubtype = USB_DESC_CS_INTERFACE_HEADER,
      .bcdCDC             = 0x0110,
    },
    .cdc_cm = {
      .bLength            = sizeof(usb_cdc_cm_desc_t),
      .bDescriptorType    = USB_DESC_CS_INTERFACE,
      .bDescriptorSubtype = USB_DESC_CS_INTERFACE_CM,
      .bmCapabilities     = 0x02,
      .bDataInterface     = 0x01,
    },
    .cdc_acm = {
      .bLength            = sizeof(usb_cdc_acm_desc_t),
      .bDescriptorType    = USB_DESC_CS_INTERFACE,
      .bDescriptorSubtype = USB_DESC_CS_INTERFACE_ACM,
      .bmCapabilities     = 0x02,
    },
    .cdc_union = {
      .bLength            = sizeof(usb_cdc_union_desc_t),
      .bDescriptorType    = USB_DESC_CS_INTERFACE,
      .bDescriptorSubtype = USB_DESC_CS_INTERFACE_UNION,
      .bMasterInterface   = 0x00,
      .bSlaveInterface0   = 0x01,
    },
    .intf0_ep0 = {
      .bLength          =    sizeof(usb_endpoint_desc_t),
      .bDescriptorType  =    USB_DESC_ENDPOINT,
      .bEndpointAddress =   0x81, // IN
      .bmAttributes     =    0x3, // Interrupt
      .wMaxPacketSize   =    16,
      .bInterval        =    0xa,
    },
  .interface1 = {
    .bLength            =    sizeof(usb_interface_desc_t),
    .bDescriptorType    =    USB_DESC_INTERFACE,
    .bInterfaceNumber   =    0x1,
    .bAlternateSetting  =    0x0,
    .bNumEndpoints      =    0x2,
    .bInterfaceClass    =    0xa, // CDC Data
    .bInterfaceSubClass =    0x0,
    .bInterfaceProtocol =    0x0,
    .iInterface         =    0x0,
  },
    .intf1_ep0 = {
      .bLength          =    sizeof(usb_endpoint_desc_t),
      .bDescriptorType  =    USB_DESC_ENDPOINT,
      .bEndpointAddress =   0x82, // IN
      .bmAttributes     =    0x2, // Bulk
      .wMaxPacketSize   =    NRF_USBD_COMMON_EPSIZE,
      .bInterval        =    0x0,
    },
    .intf1_ep1 = {
      .bLength          =    sizeof(usb_endpoint_desc_t),
      .bDescriptorType  =    USB_DESC_ENDPOINT,
      .bEndpointAddress =    0x1, // OUT
      .bmAttributes     =    0x2, // Bulk
      .wMaxPacketSize   =    NRF_USBD_COMMON_EPSIZE,
      .bInterval        =    0x0,
    },
};

// Variables needed for operation

typedef struct __attribute__ ((packed)) {
  uint32_t dwDTERate;
  uint8_t bCharFormat;
  uint8_t bParityType;
  uint8_t bDataBits;
} line_coding_t;

static atomic_t host_ready = false;
static atomic_t received_packet = false;
static line_coding_t line_coding;

static uint8_t request_data[NRF_USBD_COMMON_EPSIZE];
static nrf_usbd_common_setup_t pending_req;


static bool panic_mode = false;
static bool panic_timed_out = false;
static atomic_t tx_active = false;
static size_t tx_n;
static uint8_t rx_temp_buf[NRF_USBD_COMMON_EPSIZE];
static uint8_t rx_buf[1000];
static cb_t rx_cb = CB_INIT(rx_buf);
static cb_t* tx_cb = NULL;
struct k_event event;

// uclog sends ping packets at this rate
#define UCLOG_PING_RATE_MS 500
// ping_timeout_timer will expire if no packets received in this time
#define PING_TIMEOUT_MS (UCLOG_PING_RATE_MS * 2)

struct k_timer ping_timeout_timer;

static struct onoff_manager *hfxo_mgr;
static struct onoff_client hfxo_cli;
static atomic_t hfxo_requested = 0;


static int hfxo_stop(void) {
  if (atomic_cas(&hfxo_requested, 1, 0)) {
    LOG_INFO("hfxo release");
    return onoff_cancel_or_release(hfxo_mgr, &hfxo_cli);
  }
  return 0;
}

static void hfxo_started(void) {
  LOG_INFO("hfxo started");
}

static int hfxo_start(void) {
  if (atomic_cas(&hfxo_requested, 0, 1)) {
    LOG_INFO("hfxo request");
    sys_notify_init_callback(&hfxo_cli.notify, hfxo_started);
    return onoff_request(hfxo_mgr, &hfxo_cli);
  }
  return 0;
}

static void usb_dc_power_event_handler(nrfx_power_usb_evt_t event) {
  LOG_INFO("usb_dc_power_event_handler event:{enum:nrfx_power_usb_evt_t}%d", event);
  switch (event) {
    case NRFX_POWER_USB_EVT_DETECTED:
      LOG_INFO("USB detected");
      nrf_usbd_common_enable();
      if (hfxo_start() < 0) LOG_FATAL("hfxo_start");
      break;
    case NRFX_POWER_USB_EVT_READY:
      // Enable control end points - disable all others
      // nrf_usbd_common_ep_default_config();
      nrf_usbd_common_ep_enable(NRF_USBD_COMMON_EPIN0);
      nrf_usbd_common_ep_enable(NRF_USBD_COMMON_EPOUT0);
      nrf_usbd_common_start(false);  // No SOF
      break;
    case NRFX_POWER_USB_EVT_REMOVED:
      nrf_usbd_common_disable();
      atomic_set(&host_ready, false);
      atomic_set(&received_packet, false);
      atomic_set(&tx_active, false);
      if (hfxo_stop() < 0) LOG_FATAL("hfxo_stop");
      break;
    default:
      return;
  }
}

static void handle_get_desc(nrf_usbd_common_setup_t* req) {
  enum descriptor_type_e type = req->wValue >> 8;
  unsigned idx                = req->wValue & 0xff;
  unsigned n                  = req->wLength;

  LOG_INFO("handle_get_desc type: {enum:descriptor_type_e}%d idx: %d n: %u",
      type, idx, n);

  switch (type) {
    case USB_DESC_DEVICE:
      if (idx == 0) { // There is only 1 device
        if (sizeof(device) < n) n = sizeof(device);
        LOG_MEM_INFO("sending USB_DESC_DEVICE:", &device, n);
        NRF_USBD_COMMON_TRANSFER_IN(tx, &device, n, 0);
        nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN0, &tx);
        if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
      }
      break;
    case USB_DESC_CONFIGURATION:
      if (idx == 0) { // There is only 1 configuration
        if (sizeof(configuration) < n) n = sizeof(configuration);
        LOG_MEM_INFO("sending USB_DESC_CONFIGURATION:", &configuration, n);
        NRF_USBD_COMMON_TRANSFER_IN(tx, &configuration, n, 0);
        nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN0, &tx);
        if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
      }
      break;
    case USB_DESC_STRING:
      if ((idx != 0) && (req->wIndex != 0x0409)) {
        LOG_ERROR("unknown language for get STRING %04x", req->wIndex);
      }
      else if (idx >= (sizeof(usb_strings)/sizeof(*usb_strings))) {
        LOG_ERROR("invalid STRING index:%u", idx);
      }
      else {
        if (usb_strings[idx]->bLength < n) n = usb_strings[idx]->bLength;
        LOG_MEM_INFO("sending USB_DESC_STRING:", usb_strings[idx], n);
        NRF_USBD_COMMON_TRANSFER_IN(tx, usb_strings[idx], n, 0);
        nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN0, &tx);
        if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
      }
      break;
   case USB_DESC_DEVICE_QUALIFIER:
      if (idx == 0) { // There is only 1 device
        if (sizeof(device_qual) < n) n = sizeof(device_qual);
        LOG_MEM_INFO("sending USB_DESC_DEVICE_QUAL:", &device_qual, n);
        NRF_USBD_COMMON_TRANSFER_IN(tx, &device_qual, n, 0);
        nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN0, &tx);
        if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
      }
      break;
#if 0
    case USB_DESC_INTERFACE:
      break;
    case USB_DESC_ENDPOINT:
      break;
     case USB_DESC_OTHER_SPEED:
      break;
    case USB_DESC_INTERFACE_POWER:
      break;
#endif
    default:
      LOG_ERROR("unhandled decriptor type:  {enum:descriptor_type_e}%d", type);
      break;
  }
}

static void handle_device_setup(nrf_usbd_common_setup_t* req, const void* data) {
  switch (req->bRequest) {
    case USB_SREQ_SET_FEATURE:
      if (req->wValue == 1) { // Device wakeup
        LOG_INFO("Enabling device wakeup");
        // TODO What do I need to do?
        nrf_usbd_common_setup_clear();  // no data to receive/send so finish
      }
      else {
        LOG_ERROR("unknown feature: %u", req->wValue);
      }
      break;
    case USB_SREQ_SET_ADDRESS:
      // SET_ADDRESS is handled by HW
      break;
    case USB_SREQ_GET_DESCRIPTOR:
      handle_get_desc(req);
      break;
    case USB_SREQ_SET_CONFIGURATION:
      if (req->wValue == 1) {
        LOG_INFO("enabling end points for configuration 1");
        // We are good to go - enable end points ....
        // Note: EPIN1 is never used for this implementation
        // TODO see if can leave EPIN1 unconfigured
        nrf_usbd_common_ep_enable(NRF_USBD_COMMON_EPIN1);  // Int - control line changes
        nrf_usbd_common_ep_enable(NRF_USBD_COMMON_EPOUT1); // Bulk - host -> device
        nrf_usbd_common_ep_enable(NRF_USBD_COMMON_EPIN2);  // Bulk - device -> host]
        nrf_usbd_common_setup_clear();  // no data to receive/send so finish
      }
      else {
        LOG_ERROR("unknown configuration: %u", req->wValue);
      }
      break;
#if 0
    case USB_SREQ_SET_DESCRIPTOR:
      break;
    case USB_SREQ_GET_CONFIGURATION:
      break;
    case USB_SREQ_GET_STATUS:
      break;
    case USB_SREQ_CLEAR_FEATURE:
      break;
    case USB_SREQ_GET_INTERFACE:
      break;
    case USB_SREQ_SET_INTERFACE:
      break;
    case USB_SREQ_SYNCH_FRAME:
      break;
#endif
    default:
      LOG_ERROR("unhandled device req: {enum:setup_req_e}%u", req->bRequest);
      break;
  }
}

static void send_device_info(void) {
  // Send device info to host so it can use hash to validate log parsing
  atomic_set(&tx_active, true);

  LOG_INFO("Sending device info");
  tx_n = 0; // not peeking from tx_cb for this transfer
  NRF_USBD_COMMON_TRANSFER_IN(tx, device_info_tx_buf, device_info_len, 0);
  nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN2, &tx);
  if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
}

static void handle_class_setup(nrf_usbd_common_setup_t* req, const void* data) {
  if ((req->bmRequestType & 0x1f) == 1) {
    switch (req->bRequest) {
      case CS_SET_LINE_CODING: {
        memmove(&line_coding, data, sizeof(line_coding));
        LOG_INFO("line coding br: %u char format: %u parity: %u data bits: %u",
            line_coding.dwDTERate, line_coding.bCharFormat,
            line_coding.bParityType, line_coding.bDataBits);
        // Nothing to do as not relavent for this application
        break;
      }
      case CS_GET_LINE_CODING: {
        LOG_INFO("get line coding");
        NRF_USBD_COMMON_TRANSFER_IN(tx, &line_coding, sizeof(line_coding), 0);
        nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN0, &tx);
        if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
        break;
      }
      case CS_SET_CONTROL_LINE_STATE:
        LOG_INFO("line control dtr: %u rts: %u",
            (req->wValue & 1) != 0 ? 1 : 0,
            (req->wValue & 2) != 0 ? 1 : 0);
        bool ready = req->wValue == 3;
        nrf_usbd_common_setup_clear();  // no data to receive/send so finish

        if (ready) {
          if(atomic_get(&received_packet)) {
            send_device_info();
          }
        } else {
          // Port closed by uclog. reset received_packet
          atomic_set(&received_packet, false);
        }
        atomic_set(&host_ready, ready);
        break;
      default:
        LOG_ERROR("unhandled class req: %u", req->bRequest);
        break;
    }
  }
  else {
    LOG_ERROR("unknown class %u", req->bmRequestType & 0x1f);
  }
}

size_t usb_rx_avail(const struct device *dev) {
  (void) dev;
  return cb_peek_avail(&rx_cb);
}

const uint8_t* usb_rx_peek(const struct device *dev) {
  (void) dev;
  return cb_peek(&rx_cb);
}

void usb_rx_skip(const struct device *dev, size_t n) {
  (void) dev;
  cb_skip(&rx_cb, n);
}

int usb_set_tx_cb(const struct device *dev, cb_t* cb) {
  (void) dev;
  tx_cb = cb;
  return 0;
}

int usb_tx_schedule(const struct device *dev, const uint8_t* prefix, size_t pn) {
  (void) dev;
  if (tx_cb && atomic_get(&host_ready) && atomic_get(&received_packet)) {
    bool got = atomic_cas(&tx_active, false, true);
    if (got) {
      size_t n = cb_peek_avail(tx_cb);
      // if ((prefix != NULL) && (pn > 0)) {
      //   tx_n = 0;
      //   NRF_USBD_COMMON_TRANSFER_IN(tx, prefix, pn, 0);
      //   nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN2, &tx);
      //   if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
      // }
      // else 
      if (n > 0) {
        tx_n = n;
        NRF_USBD_COMMON_TRANSFER_IN(tx, cb_peek(tx_cb), n, 0);
        nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN2, &tx);
        if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
      }
      else {
        atomic_set(&tx_active, false);
      }
    }
    if (panic_mode && (!panic_timed_out)) {
      for (int i = 0; (i < 10000) && (cb_read_avail(tx_cb) > 0); i++) {
        nrf_usbd_common_irq_handler();
      }
      if (cb_read_avail(tx_cb) > 0) panic_timed_out = true;
    }
  }
  return 0;
}

uint32_t usb_wait_event(const struct device *dev, uint32_t mask, bool reset, k_timeout_t timeout) {
  (void) dev;
  // FIXME When Zephyr does this right!
  //
  // Need to lock as k_event_wait is broken. It doesn't allow post-clear without potential for events getting lost.
  // Not the only one to notice
  // https://github.com/zephyrproject-rtos/zephyr/issues/46117
  //
  unsigned int key = irq_lock();
  uint32_t r =  k_event_wait(&event, mask, reset, timeout);
  if (r != 0) k_event_clear(&event, r);
  irq_unlock(key);
  return r;
}

#define NRF_USBD_COMMON_EP_NUM(ep)    (ep & 0xF)
#define NRF_USBD_COMMON_EP_IS_IN(ep)  ((ep & 0x80) == 0x80)
#define NRF_USBD_COMMON_EP_IS_OUT(ep) ((ep & 0x80) == 0)
#define NRF_USBD_COMMON_EP_IS_ISO(ep) ((ep & 0xF) >= 8)

/**
 * @brief Assert endpoint number validity.
 *
 * Internal macro to be used during program creation in debug mode.
 * Generates assertion if endpoint number is not valid.
 *
 * @param ep Endpoint number to validity check.
 */
#define NRF_USBD_COMMON_ASSERT_EP_VALID(ep) __ASSERT_NO_MSG(         \
	((NRF_USBD_COMMON_EP_IS_IN(ep) &&                            \
	 (NRF_USBD_COMMON_EP_NUM(ep) < NRF_USBD_COMMON_EPIN_CNT)) || \
	 (NRF_USBD_COMMON_EP_IS_OUT(ep) &&                           \
	 (NRF_USBD_COMMON_EP_NUM(ep) < NRF_USBD_COMMON_EPOUT_CNT))));


/* Return number of bytes last transferred by EasyDMA on given endpoint */
static uint32_t usbd_ep_amount_get(nrf_usbd_common_ep_t ep)
{
	int ep_in = NRF_USBD_COMMON_EP_IS_IN(ep);
	int ep_num = NRF_USBD_COMMON_EP_NUM(ep);

	NRF_USBD_COMMON_ASSERT_EP_VALID(ep);

	if (!NRF_USBD_COMMON_EP_IS_ISO(ep_num)) {
		if (ep_in) {
			return NRF_USBD->EPIN[ep_num].AMOUNT;
		} else {
			return NRF_USBD->EPOUT[ep_num].AMOUNT;
		}
	}

	return ep_in ? NRF_USBD->ISOIN.AMOUNT : NRF_USBD->ISOOUT.AMOUNT;
}

static void usbd_event_handler(nrf_usbd_common_evt_t const *const p_event) {
  // LOG_INFO("usbd_event_handler event:{enum:nrf_usbd_common_event_type_t}%d", p_event->type);
  switch (p_event->type) {
    case NRF_USBD_COMMON_EVT_SUSPEND:
      nrf_usbd_common_suspend();
      break;
    case NRF_USBD_COMMON_EVT_RESUME:
      // Nothing to do here
      break;
    case NRF_USBD_COMMON_EVT_RESET:
      // Restart which end points are enabled
      // nrf_usbd_common_ep_default_config();
      nrf_usbd_common_ep_enable(NRF_USBD_COMMON_EPIN0);
      nrf_usbd_common_ep_enable(NRF_USBD_COMMON_EPOUT0);
      atomic_set(&host_ready, false);
      atomic_set(&received_packet, false);
      atomic_set(&tx_active, false);
      break;
    case NRF_USBD_COMMON_EVT_SETUP: {
      nrf_usbd_common_setup_t req;
      nrf_usbd_common_setup_get(&req);
      LOG_INFO("  bmRequestType: %u bRequest: {enum:setup_req_e}%u"
               " wValue: %u wIndex: %u wLength: %u",
          req.bmRequestType, req.bRequest, req.wValue, req.wIndex, req.wLength);

      if ((req.bmRequestType & 0x80) == 0x80) {
        // Device to host - dispatch
        if ((req.bmRequestType & 0x60) == 0x00) {
          handle_device_setup(&req, NULL);
        }
        else if ((req.bmRequestType & 0x60) == 0x20) {
          handle_class_setup(&req, NULL);
        }
        else {
          LOG_ERROR("unhandled request type: %u", req.bmRequestType & 0x60);
        }
      }
      else {
        // Need to read data that host is sending then process
        LOG_INFO("reading %u", req.wLength);
        if (req.wLength == 0) {
          if ((req.bmRequestType & 0x60) == 0x00) {
            handle_device_setup(&req, NULL);
          }
          else if ((req.bmRequestType & 0x60) == 0x20) {
            handle_class_setup(&req, NULL);
          }
          else {
            LOG_ERROR("unhandled request type: %u", req.bmRequestType & 0x60);
          }
        }
        else if (req.wLength <= sizeof(request_data)) {
          nrf_usbd_common_setup_data_clear();
          NRF_USBD_COMMON_TRANSFER_OUT(rx, request_data, req.wLength);
          pending_req = req;
          nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPOUT0, &rx);
          if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
        }
        else {
          LOG_ERROR("host trying to send too much data: %u", req.wLength);
        }
      }
      break;
    }
    case NRF_USBD_COMMON_EVT_EPTRANSFER:
      // LOG_INFO("  on ep: %u status: {enum:nrf_usbd_common_ep_status_t}%d",
      //     p_event->data.eptransfer.ep, p_event->data.eptransfer.status);
      if (p_event->data.eptransfer.ep == NRF_USBD_COMMON_EPIN0) {
        nrf_usbd_common_setup_clear();
      }
      else if (p_event->data.eptransfer.ep == NRF_USBD_COMMON_EPOUT0) {
        // Result of reading host to device data - dispatch
        if ((pending_req.bmRequestType & 0x60) == 0x00) {
          handle_device_setup(&pending_req, request_data);
        }
        else if ((pending_req.bmRequestType & 0x60) == 0x20) {
          handle_class_setup(&pending_req, request_data);
        }
        else {
          LOG_ERROR("unhandled request type: %u", pending_req.bmRequestType & 0x60);
        }

        // Finished processing let host know
        nrf_usbd_common_setup_clear();
      }
      else if (p_event->data.eptransfer.ep == NRF_USBD_COMMON_EPOUT1) {
        if (p_event->data.eptransfer.status == NRF_USBD_COMMON_EP_WAITING) {
          NRF_USBD_COMMON_TRANSFER_OUT(rx, rx_temp_buf, sizeof(rx_temp_buf));
          nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPOUT1, &rx);
          if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
        }
        else {
          size_t n = usbd_ep_amount_get(NRF_USBD_COMMON_EPOUT1);
          size_t na = cb_write_avail(&rx_cb);
          if (n > na) {
            LOG_ERROR("dropping rx data n:%u", n - na);
            n = na;
          }
          // LOG_INFO("received: %zu", n);
          cb_write(&rx_cb, rx_temp_buf, n);
          k_event_post(&event, UCUART_EVT_RX);

          k_timer_start(&ping_timeout_timer, K_MSEC(PING_TIMEOUT_MS), K_NO_WAIT);

          // Mark as host ready if we've received something
          bool changed = atomic_cas(&received_packet, false, true);
          if (changed && atomic_get(&host_ready)) {
            send_device_info();
          }
        }
      }
      else if (p_event->data.eptransfer.ep == NRF_USBD_COMMON_EPIN2) {
        if (p_event->data.eptransfer.status == NRF_USBD_COMMON_EP_OK) {
          if (tx_cb == NULL) {
            // No tx buffer set yet. We can get here via send_device_info() xfer
            atomic_set(&tx_active, false);
          } else {
            if (cb_peek_avail(tx_cb) < tx_n) {
              LOG_FATAL("we are trying to double read");
            }
            if (tx_n > 0) {
              cb_skip(tx_cb, tx_n);
            }
            size_t n = cb_peek_avail(tx_cb);
            bool ready = atomic_get(&host_ready);
            if ((n > 0) && ready) {
              atomic_set(&tx_active, true);
              tx_n = n;
              NRF_USBD_COMMON_TRANSFER_IN(tx, cb_peek(tx_cb), n, 0);
              nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN2, &tx);
              if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
            }
            else if ((n == 0) && (tx_n > 0U) && ((tx_n % NRF_USBD_COMMON_EPSIZE) == 0U) && ready) {
              // If last transfer was multiple of EPSIZE and we are done
              // sending - then let other end know we are done sending
              // by sending a 0 length packet
              tx_n = 0;
              NRF_USBD_COMMON_TRANSFER_IN(tx, cb_peek(tx_cb), 0, 0);
              nrfx_err_t e = nrf_usbd_common_ep_transfer(NRF_USBD_COMMON_EPIN2, &tx);
              if (e != NRFX_SUCCESS) LOG_ERROR("nrf_usbd_common_ep_transfer() %08x", e);
            }
            else {
              atomic_set(&tx_active, false);
            }
          }
        }
      }
      break;
    default:
      break;
  }
}

static void usb_rx_start(const struct device *dev) {
  (void) dev;
#if 0
  struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);
  (void) data;
#endif
}

static void usb_rx_stop(const struct device *dev) {
  (void) dev;
#if 0
  struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);
  (void) data;
#endif
}


static int usb_tx_buffer(const struct device *dev, const uint8_t* b, size_t n) {
  (void) dev;
  LOG_FATAL("b: %p n: %zu", (const void*) b, n);

#if 0
  const struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);
  if (data->tx_cb == NULL) return -EIO;
  cb_write(data->tx_cb, b, n);
#endif
  return 0;
}

static int usb_tx(const struct device *dev, const uint8_t* b, size_t n) {
  LOG_FATAL("b: %p n: %zu", (const void*) b, n);
#if 0
  const struct ucuart_data * data = ZEPHYR_DEVICE_MEMBER(dev, data);
  if (data->tx_cb == NULL) return -EIO;
  cb_write(data->tx_cb, b, n);
#endif
  return usb_tx_schedule(dev, NULL, 0);
}

static int usb_panic(const struct device* dev) {
  // Assumes interrupts are disabled from this point on
  panic_mode = true;
  return 0;
}

static int usb_is_host_ready(const struct device* dev, bool* ready) {
  ARG_UNUSED(dev);
  *ready = (atomic_get(&host_ready) == true) && (atomic_get(&received_packet) == true);
  return 0;
}

static const struct ucuart_driver_api ucusb_api = {
  .tx_no_wait = usb_tx,
  .tx_buffer = usb_tx_buffer,
  .tx_schedule = usb_tx_schedule,
  .set_tx_cb = usb_set_tx_cb,
  .rx_start = usb_rx_start,
  .rx_stop = usb_rx_stop,
  .rx_avail = usb_rx_avail,
  .rx_peek = usb_rx_peek,
  .rx_skip = usb_rx_skip,
  .wait_event = usb_wait_event,
  .panic = usb_panic,
  .is_host_ready = usb_is_host_ready,
};


static void fill_serial_number(void) {
  static const char hex[] = "0123456789ABCDEF";
  uint8_t hwid[8];
  uint8_t*sn = (uint8_t*) serial_number.unicode_string;

  memset(hwid, 0, sizeof(hwid));
  hwinfo_get_device_id(hwid, sizeof(hwid));
  LOG_MEM_INFO("sn:", hwid, sizeof(hwid));
  memset(serial_number.unicode_string, 0, sizeof(hwid) * 2);
  for (size_t i = 0; i < sizeof(hwid); i++) {
      sn[(i * 2 + 0) * 2] = hex[hwid[i] >> 4];
      sn[(i * 2 + 1) * 2] = hex[hwid[i] & 0xf];
  }
}

static void fill_device_info(void) {
  size_t port_offset = sizeof(device_info_tx_buf) - (size_t)MAX_LOG_TX_SIZE;

  // Encode port number
  uint8_t port = DEVICE_INFO_UCLOG_PORT;
  device_info_tx_buf[port_offset] = (port << 2) | 3;

  // Encode CBOR
  size_t cbor_offset = port_offset + 1;
  uint8_t *cbor_output = &device_info_tx_buf[cbor_offset];

  cbor_stream_t cbor_stream;
  cbor_init(&cbor_stream, cbor_output, MAX_LOG_TX_SIZE);

#ifdef CONFIG_SIGNED_IMAGE
  const uint8_t *app_hash__ = sbl_app_hash();
#endif

  cbor_error_t err = cbor_pack(&cbor_stream,
    "{"
        ".app_hash:b,"
        ".board:s"
    "}",
        app_hash__, APP_HASH_SIZE,
        CONFIG_BOARD
    );
  if (err != CBOR_ERROR_NONE) {
    LOG_FATAL("CBOR pack error: {enum:cbor_error_t}%d", err);
  }

  size_t cbor_len = cbor_read_avail(&cbor_stream);

  // Encode COBS
  size_t cobs_len = cobs_enc(&device_info_tx_buf[1],
      &device_info_tx_buf[port_offset],
      cbor_len + 1);

  // Frame
  device_info_tx_buf[0] = '\0';
  device_info_tx_buf[cobs_len+1] = '\0';

  device_info_len = cobs_len + 2;
}

void ping_timeout(struct k_timer *timer) {
  atomic_set(&received_packet, false);
  LOG_WARN("Ping timeout expired: Host disconnected");
}

static int usb_init(const struct device *arg) {
  nrfx_err_t err;
  LOG_INFO("usb_init");

  k_event_init(&event);
  k_timer_init(&ping_timeout_timer, ping_timeout, NULL);

  fill_serial_number();
  fill_device_info();

  hfxo_mgr = z_nrf_clock_control_get_onoff(CLOCK_CONTROL_NRF_SUBSYS_HF);

  static const nrfx_power_config_t power_config = {
          .dcdcen = IS_ENABLED(CONFIG_SOC_DCDC_NRF52X) ||
                    IS_ENABLED(CONFIG_SOC_DCDC_NRF53X_APP),
  };

  static const nrfx_power_usbevt_config_t usbevt_config = {
          .handler = usb_dc_power_event_handler
  };

  (void)nrfx_power_init(&power_config);
  nrfx_power_usbevt_init(&usbevt_config);

  IRQ_CONNECT(DT_INST_IRQN(0), DT_INST_IRQ(0, priority),
                    nrfx_isr, nrf_usbd_common_irq_handler, 0);

  if (nrf_usbd_common_is_enabled()) nrf_usbd_common_disable();
  if (nrf_usbd_common_is_initialized()) nrf_usbd_common_uninit();

  err = nrf_usbd_common_init(usbd_event_handler);
  if (err < 0) LOG_FATAL("nrf_usbd_common_init() = %08x", err);

  nrfx_power_usbevt_enable();

  // If already connected then force the detected event as it otherwise
  // won't happen.
  //if (nrfx_power_usbstatus_get() != NRFX_POWER_USB_STATE_DISCONNECTED) {
  //   usb_dc_power_event_handler(NRFX_POWER_USB_EVT_DETECTED);
  //}

  return 0;
}


#define UCUSB_DEFINE(i)                                             \
  DEVICE_DT_INST_DEFINE(i, usb_init, NULL, NULL, NULL, \
      POST_KERNEL, CONFIG_UCUSB_INIT_PRIORITY, &ucusb_api);

DT_INST_FOREACH_STATUS_OKAY(UCUSB_DEFINE)
