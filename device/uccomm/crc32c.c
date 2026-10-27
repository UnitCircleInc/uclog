// © 2025 Unit Circle Inc.
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

// Implementation of CRC32C
// See:
//   http://users.ece.cmu.edu/~koopman/roses/dsn04/koopman04_crc_poly_embedded.pdf
//   http://users.ece.cmu.edu/~koopman/crc/crc32.html

#include <stddef.h>
#include <stdint.h>
#include <crc32c.h>
#include <crc32c_tab.h>

uint32_t crc32c_update(uint32_t crc, const uint8_t* data, size_t n) {
  crc ^= 0xffffffff;
  while (n-- > 0) {
    uint8_t idx = (crc & 0xff) ^ *data++;
    crc = (crc >> 8) ^ crc32c_tab[idx];
  }
  return crc ^ 0xffffffff;
}
