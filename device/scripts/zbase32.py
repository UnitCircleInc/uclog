# © 2025 Unit Circle Inc.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# A simple implementation of zbase32
# See: https://philzimmermann.com/docs/human-oriented-base-32-encoding.txt
#
# Which is based on RFC3548
# See: https://datatracker.ietf.org/doc/html/rfc3548
#
# Using a modified character set to improve readability and assumes
# encoding/decoding is performed on an integral number of bytes.
#
# An extention on zbase32 is to split encoded data into 5 character
# chunks separated by hyphens to further improve readability.
#
# Result of encode is URL safe.

from textwrap import wrap

zbase32 = "ybndrfg8ejkmcpqxot1uwisza345h769"
rev_zbase32 = {k: v for v, k in enumerate(zbase32)}


def encode(b):
    nbits = 0
    v = 0
    i = 0
    r = ""
    lb = len(b)
    while True:
        if nbits < 5 and i < lb:
            v = (b[i] << nbits) | v
            i = i + 1
            nbits = nbits + 8
        if nbits <= 0:
            break
        r = r + zbase32[v & 0x1F]
        v = v >> 5
        nbits -= 5
    return "-".join(wrap(r, 5))


def decode(s):
    s = s.replace("-", "").lower()
    b = b""
    nbits = 0
    v = 0
    i = 0
    ls = len(s)
    while True:
        while nbits < 8 and i < ls:
            v = (rev_zbase32[s[i]] << nbits) | v
            i += 1
            nbits += 5
        if nbits < 8:
            if v != 0:
                raise Exception("Invalid stream")
            break
        b = b + bytes([v & 0xFF])
        v = v >> 8
        nbits -= 8
    return b
