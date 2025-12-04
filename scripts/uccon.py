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
# limitations under the License.import threading

import threading
import queue
from uclog import StreamClient
import argparse

def background(device):
    while True:
        t = device.rx()
        if t:
            print(t.decode(), end='', flush=True)

import sys, tty, termios

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", help="host:port to use when connecting to server")
    parser.add_argument("--target", help="serial port to use when connecting")

    args = parser.parse_args()
    args.raw = False
    with StreamClient(args, stream=0, cbor_wrap=False) as device:
        threading1 = threading.Thread(target=background, args=(device,))
        threading1.daemon = True
        threading1.start()

        fd = sys.stdin.fileno()
        old_settings = termios.tcgetattr(fd)
        try:
            tty.setraw(sys.stdin.fileno())
            while True:
                ch = sys.stdin.read(1).encode()
                if ch == b'\x03':
                    break
                if ch == b'\r':
                    ch = b'\r\n'
                print(ch.decode(), end='', flush=True)
                device.tx(ch)
        finally:
            termios.tcsetattr(fd, termios.TCSADRAIN, old_settings)
