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

# Implementation of COBS - http://www.stuartcheshire.org/papers/COBSforToN.pdf


def enc(data):
    out = b""
    data = data + b"\0"  # Add "fake" zero
    while len(data) > 0:
        i = data.index(b"\0")
        if i >= 254:
            out, data = out + bytes((255,)) + data[:254], data[254:]

            # Early exit if we only have the added "fake" zero
            # No need to send extra \x01 byte - receiver can infer
            if data == b"\x00":
                break
        else:
            out, data = out + bytes((i + 1,)) + data[:i], data[i + 1 :]
    return out


def dec(data):
    out = b""
    while len(data) > 0:
        code = data[0]
        seg, data = data[1:code], data[code:]
        if code == 255 and len(data) == 0:
            # Add back "fake" zero removed by sender
            seg += b"\0"
        elif code < 255:
            seg += b"\0"
        out += seg
    return out[:-1]  # Remove "fake" zero


# Possibly faster version if compiling python to C as pre-allocates output
# TODO might be better to use bytearray instead of list.
def dec_fast(dd):
    data, idx, dellist = list(dd), 0, []
    n = len(data)
    out = [0] * (n + 1)
    out[:-1] = data
    while idx < n:
        code = data[idx]
        if code == 255 and idx == len(data):
            pass  # Zero at end already added
        elif code < 255:
            out[idx + code] = 0
        else:
            dellist.append(idx + code)
        idx += code
    r = bytes(out[1:-1])
    dellist.reverse()
    for idx in dellist:
        r = r[: idx - 1] + r[idx:]
    return r


# TODO Remove extra "fake" zero if we end on a length == 0xdf
# similar to enc/dec above.
def enc_zpe(data):
    out = b""
    data = data + b"\0"  # Add "fake" zero
    while len(data) > 0:
        i = data.index(b"\0")
        if i >= 0xDF:
            out, data = out + bytes((0xE0,)) + data[:0xDF], data[0xDF:]
        elif len(data) >= i + 2 and data[i + 1] == 0 and i <= 30:
            out, data = out + bytes((i + 0xE1,)) + data[:i], data[i + 2 :]
        else:
            out, data = out + bytes((i + 1,)) + data[:i], data[i + 1 :]
    return out


def dec_zpe(data):
    out = b""
    while len(data) > 0:
        code = data[0]
        if code < 0xE0:
            seg, data = data[1:code] + b"\0", data[code:]
        elif code == 0xE0:
            seg, data = data[1:code], data[code:]
        else:
            seg, data = data[1 : code - 0xE0] + b"\0\0", data[code - 0xE0 :]
        out += seg
    return out[:-1]  # Remove "fake" zero
