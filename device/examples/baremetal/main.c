// © 2026 Unit Circle Inc.
// SPDX-License-Identifier: Apache-2.0
//
// A minimal ucLog app, bare metal: a log every second, and a terminal (port 61) with two
// commands. Connect with any host, e.g. `uclog tui`, and type `help` in the Terminal view.

#include <stdint.h>
#include <string.h>

#include "log.h"
#include "port.h"

#define TERM_PORT 61 // the terminal (spec/protocol.md §5.7)

typedef enum { LED_OFF, LED_ON, LED_BLINK } led_mode_t;

static led_mode_t led_mode = LED_BLINK;

static void reply(const char *s) {
  log_tx(TERM_PORT, (const uint8_t *)s, strlen(s));
}

// A line from the terminal (the hosts send whole lines, ending in \r). Handlers run in
// log_poll(), in the main loop.
static void on_terminal(const uint8_t *rx, size_t n, void *ctx) {
  static char line[32];
  static size_t len;
  for (size_t i = 0; i < n; i++) {
    char c = (char)rx[i];
    if (c != '\r' && c != '\n') {
      if (len < sizeof(line) - 1) line[len++] = c;
      continue;
    }
    line[len] = 0;
    len = 0;
    if (line[0] == 0) continue;
    if (strcmp(line, "led on") == 0) led_mode = LED_ON;
    else if (strcmp(line, "led off") == 0) led_mode = LED_OFF;
    else if (strcmp(line, "led blink") == 0) led_mode = LED_BLINK;
    else {
      reply("commands: led on, led off, led blink\r\n");
      continue;
    }
    LOG_INFO("LED mode {enum:led_mode_t}%d", led_mode); // shown as LED_ON, LED_OFF or LED_BLINK
    reply("ok\r\n");
  }
}

int main(void) {
  log_pre_init();                      // logs are kept from here, before the link is up
  board_init();
  log_init("nucleo_l452re_p", "example"); // the board and image type, sent in the device info
  log_notify(TERM_PORT, on_terminal, NULL);
  LOG_INFO("hello from %s", "ucLog");

  uint32_t next = 0;
  uint32_t count = 0;
  for (;;) {
    log_poll(); // frames from the host: the terminal handler above
    uint32_t now = board_ms();
    if ((int32_t)(now - next) >= 0) {
      next = now + 1000;
      count++;
      LOG_INFO("tick %u, uptime %.1f s", (unsigned)count, now / 1000.0);
      if (count % 10 == 0) LOG_WARN("%u ticks without a reset", (unsigned)count);
      if (led_mode == LED_BLINK) board_led(count & 1);
    }
    if (led_mode != LED_BLINK) board_led(led_mode == LED_ON);
  }
}
