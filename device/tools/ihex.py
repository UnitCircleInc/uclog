#! /usr/bin/env python3

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

# See https://en.wikipedia.org/wiki/Intel_HEX for approx. spec.
# NOTE: arm-none-eabi-objcopy use DOS line endings on all platforms so we copy.
# NOTE: does not support comment (or similar) extensions.

import struct
import io

# Record types
TY_DATA = 0
TY_EOF = 1
TY_EXT_SEG = 2
TY_START_SEG = 3
TY_EXT_LIN = 4
TY_START_LIN = 5


def dec_record(n, rec):
    try:
        if rec[0] != ":":
            raise ValueError("missing :")
        payload = bytes.fromhex(rec[1:])
        if payload[0] != len(payload) - 5:
            raise ValueError("invalid length")
        if sum(payload) % 256 != 0:
            raise ValueError("cs fail")
        addr, ty, data = payload[1:3], payload[3], payload[4:-1]
        (addr,) = struct.unpack(">H", addr)
        if ty != TY_DATA and addr != 0:
            raise ValueError("invalid addr")
    except IndexError:
        raise ValueError(f'invalid input "{rec}" on line {n}')
    except ValueError:
        raise ValueError(f'invalid input "{rec}" on line {n}')
    return addr, ty, data


def enc_record(addr, ty, data):
    payload = struct.pack(">BHB", len(data), addr, ty) + data
    return ":" + (payload + bytes([(256 - (sum(payload) % 256)) % 256])).hex().upper()


def load(f):
    image = []
    ss = 0
    base = 0
    for n, line in enumerate(f.readlines()):
        line = line.strip()
        if len(line) == 0:
            continue
        addr, ty, data = dec_record(n, line)
        if ty == TY_DATA:
            image.append((base + addr, data))
        elif ty == TY_EOF:
            break
        elif ty == TY_EXT_SEG:
            base = struct.unpack(">H", data)[0] * 16
        elif ty == TY_START_SEG:
            cs, ip = struct.unpack(">HH", data)
            ss = cs * 16 + ip
        elif ty == TY_EXT_LIN:
            (base,) = struct.unpack(">I", data + b"\x00\x00")
        elif ty == TY_START_LIN:
            (ss,) = struct.unpack(">I", data)

    # Combine individual data items into minimal set of disjoint segments
    image = sorted(image, key=lambda x: x[0])
    addr, data = image[0]
    image2 = []
    for a, d in image[1:]:
        if addr + len(data) != a and len(data) > 0:
            image2.append((addr, data))
            addr = a
            data = b""
        data = data + d
    image2.append((addr, data))
    return ss, image2  # [(addr, data)]


def loads(s):
    f = io.StringIO(s)
    ss, image = load(f)
    f.close()
    return ss, image


def dump(f, ss, image):
    base = 0
    for addr, data in image:
        offset = addr - base
        while len(data) > 0:
            n = 16
            if offset >= 65536:
                tmp = base + offset
                base = (tmp // 65536) * 65536
                offset = tmp - base
                if base >= 65536 * 16:
                    print(
                        enc_record(0, TY_EXT_LIN, struct.pack(">I", base)),
                        file=f,
                        end="\r\n",
                    )
                else:
                    print(
                        enc_record(0, TY_EXT_SEG, struct.pack(">H", base // 16)),
                        file=f,
                        end="\r\n",
                    )
            if offset + n > 65536:
                n = 65536 - offset
            d, data = data[:n], data[n:]
            print(enc_record(offset, TY_DATA, d), file=f, end="\r\n")
            offset += n

    if ss >= 65536 * 16:
        print(enc_record(0, TY_START_LIN, struct.pack(">I", ss)), file=f, end="\r\n")
    else:
        cs = (ss // 65536) * 4096
        ip = ss - cs * 16
        print(
            enc_record(0, TY_START_SEG, struct.pack(">HH", cs, ip)), file=f, end="\r\n"
        )
    print(enc_record(0, TY_EOF, b""), file=f, end="\r\n")


def dumps(ss, image):
    f = io.StringIO()
    dump(f, ss, image)
    s = f.getvalue()
    f.close()
    return s
