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

// Implementation of CBOS - http://www.stuartcheshire.org/papers/COBSforToN.pdf

// Note:
// It is possible to encode "inplace" by placing the input data at the
// end of the buffer, cobs_enc will overwrite the input data as it encodes.
// You need to ensure the buffer is cobs_enc_size longer than the input.
// This way the write will not catch up with the read.
//
// Note:
// It is possible to decode "inplace" as the read pointer advances
// faster than the write pointer.


#include <stdbool.h>
#include <string.h>
#include "cobs.h"

size_t cobs_enc_size(size_t n) {
  return (n + 253)/254 + n;
}

size_t cobs_enc(uint8_t* out, const uint8_t* in, size_t n) {
  size_t nout = 0;
  bool last_max = false;
  out[0] = 1;
  while (n-- > 0) {
    last_max = false;
    uint8_t v = *in++;
    if (v == 0) {
      nout += out[0];
      out += out[0];
      out[0] = 1;
    }
    else {
      out[out[0]++] = v;
      if (out[0] == 255) {
        nout += out[0];
        out += out[0];
        out[0] = 1;
        last_max = true;
      }
    }
  }
  if (!last_max) {
    // Implicit 0x00 terminator - so output last segment
    nout += out[0];
  }
  return nout;
}

ssize_t cobs_dec(uint8_t* out, const uint8_t* in, size_t n) {
  bool out0 = false;
  uint8_t code = 0;
  size_t nout = 0;
  if (memchr(in, 0x00, n) != NULL) return -1; // Input must not contain 0x00
  while (n > 0) {
    if (code == 0) {
      if (out0) { *out++ = 0x00; nout++; }
      code = *in++;  n--; // Code can't be 0x00 - memchr test above
      out0 = code != 255;
      code--;
    }
    else {
      *out++ = *in++;
      nout++; n--;
      code--;
    }
  }
  if (code >  0) return -2;  // Insufficient input to decode last segment
  return nout;
}
