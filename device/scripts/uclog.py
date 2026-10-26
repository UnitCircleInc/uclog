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

import threading
import queue
import argparse
import struct
import time
import socket
import sys
import os
import select
import fnmatch
import logging

import cbor2
import serial
import serial.tools.list_ports as lp

import cobs

try:
    from logdata import LogData, TARGET_DIGIT_SHIFT, LOG_TYPE_PORT
except ModuleNotFoundError:
    pass

logging.basicConfig(filename="uclog.log", level=logging.DEBUG)

# Set the largest port/stream that is supported by python side
# This is driven by `ulimit -n`.  Default on macOS is 256 which prevents
# using 64.  Pratically the an application is not likely to use more than 8.
LOG_PORT_MAX = 8
LOG_DEFAULT_HOST = "localhost"
LOG_DEFAULT_BASE = 9000

DEFAULT_BR = 1000000  # 115200

# Monkey patch cbor to change default flags/config
orig_dumps = cbor2.dumps


def my_dumps(*args, **kwargs):
    if "datetime_as_timestamp" not in kwargs:
        kwargs["datetime_as_timestamp"] = True
    return orig_dumps(*args, **kwargs)


cbor2.dumps = my_dumps

# Ensure we get ANSI console escape sequences
if sys.platform == "win32":
    import ctypes

    kernel32 = ctypes.windll.kernel32
    kernel32.SetConsoleMode(kernel32.GetStdHandle(-11), 7)

# TODO perhaps have the data passed between items be a tupple
# ('ok'/'error', value/errormsg)
# Or perhaps using of ideas in https://github.com/matthewgdv/maybe
# That way decode errors could be propagated to display.
# Right now errors are silently dropped making debugging more
# difficult.
# Can also set up to just print errors to a global function that knows
# which display to send things to.
# Or use standard python logging framework


class CobsDecode(object):
    def __init__(self):
        self.indata = b""
        self.on_data = None

    def __call__(self, data):
        self.indata = self.indata + data
        if len(self.indata) > 1500 + 20:
            self.indata = self.indata[: 1500 + 20]
        while b"\x00" in self.indata:
            frame, self.indata = self.indata.split(b"\x00", 1)
            try:
                if self.on_data and len(frame) > 0:
                    self.on_data(cobs.dec(frame))
            except Exception:
                logging.error("exception ", exc_info=1)


class CobsEncode(object):
    def __init__(self):
        self.on_data = None

    def __call__(self, data):
        if self.on_data:
            self.on_data(b"\x00" + cobs.enc(data) + b"\x00")


class CborDecode(object):
    def __init__(self):
        self.on_data = None

    def __call__(self, data):
        try:
            dec = cbor2.loads(data)
        except cbor2.CBORDecodeError:
            return
        if self.on_data:
            self.on_data(dec)


class CborEncode(object):
    def __init__(self):
        self.on_data = None

    def __call__(self, data):
        if self.on_data:
            self.on_data(cbor2.dumps(data))


class MuxDecode(object):
    def __init__(self, on_data):
        self.on_data = on_data

    def __call__(self, frame):
        if len(frame) == 0:
            return
        p, t = divmod(int(frame[0]), 4)
        if t == LOG_TYPE_PORT:
            if p in self.on_data:
                self.on_data[p](frame[1:])
            elif p == 63:
                print(f"----- Image hash: {frame[1:].hex()} -----")
        elif len(frame) >= 4:
            addr, frame = struct.unpack("<I", frame[:4])[0], frame[4:]
            target = (addr >> TARGET_DIGIT_SHIFT) & 0xF
            if "log" in self.on_data:
                self.on_data["log"]((target, addr, frame))
        else:
            if "error" in self.on_data:
                self.on_data["error"](frame)


class MuxEncode(object):
    def __init__(self, port):
        self.port = port
        self.on_data = None

    def __call__(self, frame):
        if self.on_data:
            self.on_data(bytes(((self.port << 2) | LOG_TYPE_PORT,)) + frame)


class LogDecode(object):
    def __init__(self, dec):
        self.dec = dec
        self.on_data = None

    def __call__(self, item):
        try:
            target, _, _ = item
            if target in self.dec:
                r = self.dec[target].decode(item)
            else:
                r = item
        except Exception:
            logging.error("exception ", exc_info=1)
            r = item
        if self.on_data:
            self.on_data(r)


class LogDisplay(object):
    def __init__(self):
        pass

    def __call__(self, item):
        if len(item) == 5:
            count, ts, target, addr, frame = item
            print(f"{count} {ts} {target} {hex(addr)} {frame.hex()}")
            # print((count, ts, target, addr, frame))
        elif len(item) == 6:
            count, ts, level, file, line, text = item
            file = file if len(file) <= 30 else "... " + file[-26:]
            print(f"{ts:10.3f}:{level:5}:{file:30s}:{line:3}:{text}")
            # print((count, ts, level.strip(), file, int(line), text))
        else:
            print(item)


class Network(threading.Thread):
    def __init__(self, addr):
        threading.Thread.__init__(self)
        self.addr = addr
        self.on_data = None
        self.alive = True
        self.wake_write, self.wake_read = socket.socketpair()
        self.lock = threading.Lock()
        self.start()

    def shutdown(self):
        if self.alive:
            self.alive = False
            self.wake_write.sendall(b"Wakeup")
            self.wake_write.close()
            self.join()

    def __call__(self, data):
        with self.lock:
            if self.conn:
                self.conn.sendall(data)

    def process(self):
        while self.alive:
            r, _, _ = select.select((self.conn, self.wake_read), (), (), 0.1)
            for z in r:
                if z == self.conn:
                    data = self.conn.recv(4096)
                    if len(data) == 0:
                        logging.debug(f"peer closed connection {self.conn}")
                        return

                    if self.on_data:
                        self.on_data(data)
                elif z == self.wake_read:
                    # Local request to terminate
                    self.wake_read.recv(4096)
                    self.wake_read.close()


class Client(Network):
    def __init__(self, addr):
        self.failed_to_connect = False
        Network.__init__(self, addr)

    def run(self):
        self.conn = None
        self.connected = False
        try:
            logging.debug(f"client connecting to: {self.addr}")
            self.conn = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.conn.connect(self.addr)
            self.connected = True
            self.process()
        except Exception:
            self.failed_to_connect = True
            # logging.error("exception ",exc_info=1)
        finally:
            self.connected = False
            logging.debug(f"client closing: {self.addr}")
            if self.conn:
                self.conn.close()


class Server(Network):
    def __init__(self, addr):
        Network.__init__(self, addr)

    def run(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.conn = None
        try:
            logging.debug(f"server starting on: {self.addr}")
            sock.bind(self.addr)
            sock.listen(1)  # Each server can only have one client
            while self.alive:
                try:
                    r, _, _ = select.select((sock, self.wake_read), (), (), 0.1)
                    for z in r:
                        if z == sock:
                            self.conn, self.client = sock.accept()
                            logging.debug(
                                f"accepting connection on: {self.addr} from: {self.client}"
                            )
                            self.process()
                        elif z == self.wake_read:
                            # Local request to terminate
                            self.wake_read.recv(4096)
                            self.wake_read.close()
                except Exception as e:
                    logging.error("exception ", exc_info=1)
                    raise e
                finally:
                    with self.lock:
                        if self.conn:
                            logging.debug(
                                f"closing connection on: {self.addr} from: {self.client}"
                            )
                            self.conn.close()
                        self.conn = None
        finally:
            logging.debug(f"server stopping on: {self.addr}")
            sock.close()


class Serial(threading.Thread):
    def __init__(self, dev, baudrate, status_change_cb=None):
        threading.Thread.__init__(self)
        self.dev = dev
        self.on_data = None
        self.baudrate = baudrate
        self.serial = serial.Serial(
            self.dev, baudrate=self.baudrate, stopbits=1, timeout=0.1
        )
        self.alive = True
        self.lock = threading.Lock()
        self.last_send = time.time() - 1
        self.cnt = 0
        self.status_change_cb = status_change_cb

    def shutdown(self):
        if self.alive:
            self.alive = False
            self.join()
            self.serial.close()

    def __call__(self, data):
        # Ensure each "packet" is fully sent before the next one
        with self.lock:
            self.last_send = time.time()
            # print(f'---> {data.hex()}')
            # STLINK seems to have bug if len(data) % 8 == 0 - work around
            # This is a hack as it does not handle two calls that are close
            # together and total length % 8 == 0.  Assumes items are well spaced.
            # TODO re-test as this bug was on old STLINK rev - may have been fixed.
            if len(data) % 8 == 0:
                if len(data) - 1 != self.serial.write(data[:-1]):
                    logging.error("Error sending cmd to target")
                data = data[-1:]
                time.sleep(0.01)  # to "force" flushing to USB
            if len(data) != self.serial.write(data):
                logging.error("Error sending cmd to target")

    def send_pulse(self):
        if time.time() >= self.last_send + 0.5:
            # print("sending pulse")
            # self.cnt = (self.cnt + 1) % 256
            # self(bytes((self.cnt,)))
            self(b'\x00')
            pass

    def set_target_status(self, target_online, target_device=""):
        # Call the connection status callback
        if self.status_change_cb is not None:
            self.status_change_cb(target_online, target_device)

    def run(self):
        while self.alive:
            try:
                c = self.serial.read()
                self.send_pulse()
                if len(c) > 0:
                    # print(f'<--- {c.hex()}')
                    if self.on_data:
                        self.on_data(c)
                    else:
                        print(f"dropped (no on_data): {c.hex()}")
            except (serial.serialutil.SerialException, OSError):
                print("connection lost ...", end="", flush=True)
                self.serial.close()

                # Indicate target is offline
                self.set_target_status(target_online=False)

                while self.alive:
                    try:
                        time.sleep(0.1)
                        self.serial = serial.Serial(
                            self.dev, baudrate=self.baudrate, stopbits=1, timeout=0.1
                        )
                        print("\r... connection restored", flush=True)

                        # Indicate connection is restored
                        self.set_target_status(
                            target_online=True, target_device=self.dev
                        )

                        break
                    except (serial.serialutil.SerialException, OSError):
                        pass


# Utitlity to chain a list of processes together
def chain(items):
    for src, dst in zip(items[:-1], items[1:], strict=True):
        src.on_data = dst
    return items[0]


def target(t, raw=False):
    if t:
        return t
    else:
        if raw:
            st = [x[0] for x in lp.grep("0403:6001")]
        else:
            st = [x[0] for x in lp.grep("1366:1015")]
            # st = [x[0] for x in lp.grep('0483:374E')]
        if len(st) == 1:
            return st[0]
        elif len(st) == 0:
            print("Can't find serial port for target")
        else:
            print("Too many serials ports for target")
        for p in lp.comports():
            print(f"{p[0]} {p[1]} {p[2]}")
        exit(1)


def hostport(h):
    if h is None:
        return LOG_DEFAULT_HOST, LOG_DEFAULT_BASE
    elif ":" in h:
        host, base = h.split(":", 2)
        if host == "":
            host = LOG_DEFAULT_HOST
        if base == "":
            base = LOG_DEFAULT_BASE
        else:
            base = int(base)
        return host, base
    else:
        return host, LOG_DEFAULT_BASE


class Target(threading.Thread):
    def __init__(self, target, baudrate, status_change_cb=None):
        threading.Thread.__init__(self)
        self.threads = {}
        self.alive = True
        self.threads["serial"] = Serial(
            target, baudrate, status_change_cb=status_change_cb
        )
        self.init()
        self.threads["serial"].start()  # needs to be after self.init()
        self.start()

    def init(self):
        pass

    def shutdown(self):
        self.alive = False
        for _, thread in self.threads.items():
            thread.shutdown()

    def run(self):
        while self.alive:
            time.sleep(0.1)
            if any([not thread.is_alive() for _, thread in self.threads.items()]):
                break

    # TODO: Seems unused. Remove?
    # class Device(Target):
    #     def __init__(self, target, ondata):
    #         self.ondata = ondata
    #         Target.__init__(self, target)
    #
    #     def init(self):
    #         try:
    #             self.tx = chain(CobsEncode(), self.threads["serial"])
    #             self.rx = chain([self.threads["serial"], CobsDecode(), self.ondata])
    #             super().init()
    #         except Exception as e:
    #             self.shutdown()
    #             raise e
    #
    #     def run(self):
    #         while self.alive:
    #             time.sleep(0.1)
    #             if any([not thread.is_alive() for _, thread in self.threads.items()]):
    #                 break

    def __call__(self, data):
        self.tx(data)


class LogServer(Target):
    def __init__(self, target, hostport, decoders, baudrate=DEFAULT_BR, display=None):
        self.hostport = hostport
        self.decoders = decoders
        self.display = display
        Target.__init__(self, target, baudrate)

    def init(self):
        try:
            host, port = self.hostport
            self.threads.update(
                {i: Server((host, port + i + 1)) for i in range(LOG_PORT_MAX)}
            )
            self.rx = {
                i: chain([CobsEncode(), self.threads[i]]) for i in range(LOG_PORT_MAX)
            }
            self.tx = {
                i: chain(
                    [
                        self.threads[i],
                        CobsDecode(),
                        MuxEncode(i),
                        CobsEncode(),
                        self.threads["serial"],
                    ]
                )
                for i in range(LOG_PORT_MAX)
            }
            if self.display:
                self.rx["log"] = chain([LogDecode(self.decoders), self.display])
            else:
                self.threads["log"] = Server((host, port))
                self.rx["log"] = chain(
                    [
                        LogDecode(self.decoders),
                        CborEncode(),
                        CobsEncode(),
                        self.threads["log"],
                    ]
                )

            self.rx = chain([self.threads["serial"], CobsDecode(), MuxDecode(self.rx)])
            super().init()
        except Exception as e:
            self.shutdown()
            raise e


class LogClient(threading.Thread):
    def __init__(self, hostport, rx):
        threading.Thread.__init__(self)
        self.hostport = hostport
        self.rx = rx
        self.alive = True
        self.init()
        self.start()

    def shutdown(self):
        self.alive = False
        for _, thread in self.threads.items():
            thread.shutdown()

    def init(self):
        try:
            host, port = self.hostport
            self.threads = {
                i: Client((host, port + i + 1))
                for i in self.rx.keys()
                if i not in ["log"]
            }

            self.tx = {
                i: chain([CobsEncode(), self.threads[i]])
                for i in self.rx.keys()
                if i not in ["log"]
            }
            rx = self.rx.copy()
            self.rx = {
                i: chain([self.threads[i], CobsDecode(), v])
                for i, v in rx.items()
                if i not in ["log"]
            }
            if "log" in rx:
                self.threads["log"] = Client((host, port))
                self.rx["log"] = chain(
                    [self.threads["log"], CobsDecode(), CborDecode(), rx["log"]]
                )

            start = time.time()
            while time.time() - start < 5:
                if self.ready() or self.failed_to_connect():
                    break
                time.sleep(0.1)
            if not self.ready():
                raise Exception("unable to connect to service")
            logging.debug(
                f"took {time.time() - start:10.3f} to connect to server: {self.hostport}"
            )

        except Exception as e:
            self.shutdown()
            raise e

    def run(self):
        while self.alive:
            time.sleep(0.1)
            if any([not thread.is_alive() for _, thread in self.threads.items()]):
                break

    def ready(self):
        return all(
            [
                thread.is_alive() and thread.connected
                for _, thread in self.threads.items()
            ]
        )

    def failed_to_connect(self):
        return any([thread.failed_to_connect for _, thread in self.threads.items()])

    def __getitem__(self, key):
        """
        Returns a callable to allow sending data to a stream
        """
        return self.tx[key]


class LogClientServer(Target):
    def __init__(self, target, decoders, rx):
        self.decoders = decoders
        self.rx = rx
        Target.__init__(self, target)

    def start(self):
        try:
            self.tx = {
                i: chain([MuxEncode(i), CobsEncode(), self.threads["serial"]])
                for i in range(LOG_PORT_MAX)
            }
            rx = self.rx.copy()
            if "log" in rx:
                self.rx["log"] = chain([LogDecode(self.decoders), rx["log"]])
            self.rx = chain([self.threads["serial"], CobsDecode(), MuxDecode(self.rx)])
        except Exception as e:
            self.shutdown()
            raise e

    def ready(self):
        return True

    def __getitem__(self, key):
        """
        Returns a callable to allow sending data to a stream
        """
        return self.tx[key]


class StreamClient(object):
    """
    Simple client that only supports single cmd/rsp stream that is cbor coded.
    Stream defaults to 1 - if args.raw, then assumes no log multiplexing.
    Exposes sc.txrx(), sc.tx() and sc.rx() API.
    Expects args argument to have attributes target, host, raw
    """

    def __init__(self, args, stream=1, cbor_wrap=True):
        self.args = args
        self.stream = stream
        self.service = None
        self.cbor_wrap = cbor_wrap

    def __exit__(self, type, value, traceback):
        if self.service:
            self.service.shutdown()
        return False  # Do not suppress exception

    def __enter__(self):
        rx = {self.stream: self._ondata}

        if self.args.target:
            d = target(self.args.target, self.args.raw)
        elif self.args.host and not self.args.raw:
            d = hostport(self.args.host)
        elif log_server_active(hostport(None), streams=rx.keys()) and not self.args.raw:
            d = hostport(None)
        else:
            d = target(None, self.args.raw)

        if isinstance(d, tuple):
            self.service = LogClient(d, rx)
        elif isinstance(d, str) and self.args.raw:
            self.service = Target(d)
            self.txc = chain([CobsEncode(), self.service.threads["serial"]])
            self.rxc = chain(
                [self.service.threads["serial"], CobsDecode(), self._ondata]
            )
            self.stream = None
            self.service.threads["serial"](b"\x00")
            time.sleep(0.1)
        else:
            self.service = LogClientServer(d, [], rx)
        self.queue = queue.Queue()
        return self

    def tx(self, data):
        if self.cbor_wrap:
            self.service[self.stream](cbor2.dumps(data))
        else:
            self.service[self.stream](data)

    def _ondata(self, data):
        self.queue.put(data)

    def rx(self, timeout=0.1):
        try:
            data = self.queue.get(timeout=timeout)
            if len(data) > 0:
                if self.cbor_wrap:
                    return cbor2.loads(data)
                else:
                    return data
            else:
                return None
        except queue.Empty:
            return None

    def txrx(self, data, timeout=0.1):
        self.tx(data)
        return self.rx(timeout)


def log_server_active(hostport, streams):
    host, base = hostport
    for s in streams:
        conn = None
        hostport = (host, base + 1 + s)
        try:
            conn = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            conn.connect(hostport)
        except Exception:
            return False
        finally:
            if conn:
                conn.close()
    return True


def decoders(fnames):
    if fnames is None:
        # Then auto add ../bin-xx/yyyyyyyyy.elf
        fnames = []
        for root, _, filenames in os.walk(".."):
            if root.startswith(os.path.join("..", "bin-")):
                for filename in fnmatch.filter(filenames, "*.elf"):
                    fnames.append(os.path.join(root, filename))

    dec = [LogData(f) for f in fnames]
    return {d.target(): d for d in dec}


if __name__ == "__main__":
    parser = argparse.ArgumentParser("LOG server/viewer")
    parser.add_argument("--target", help="use serial interface when connecting")
    parser.add_argument("--host", help="use host:port when serving/connecting")
    parser.add_argument(
        "--baudrate", help="baudrate of serial interface", default=DEFAULT_BR
    )
    parser.add_argument("-s", action="store_true", help="server only mode")
    parser.add_argument("-c", action="store_true", help="client")
    parser.add_argument("-e", action="append", help="ELF to use for decoding")

    args = parser.parse_args()
    if args.s:
        o = LogServer(
            target(args.target),
            hostport(args.host),
            decoders(args.e),
            baudrate=args.baudrate,
        )
    elif args.c:
        o = LogClient(hostport(args.host), {"log": LogDisplay()})
    else:
        o = LogServer(
            target(args.target),
            hostport(args.host),
            decoders(args.e),
            display=LogDisplay(),
            baudrate=args.baudrate,
        )
    try:
        while True:
            time.sleep(0.1)
    except KeyboardInterrupt:
        pass
    except Exception:
        logging.error("exception ", exc_info=1)
    finally:
        o.shutdown()
