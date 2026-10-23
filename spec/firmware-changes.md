# Firmware changes for protocol v1 (Phase 0)

The firmware is not in this repository. This is the list of changes the firmware team needs to
make, or confirm, so that it matches [protocol.md](protocol.md). Hosts are built against these
behaviours, and the vectors in [vectors/](vectors/README.md) can be used in firmware unit tests.

Port 63 uses the **ucLog semihost op set implemented by the legacy host**
(`uclog/protocol/semihost.py`), plus four time ops. It is not ARM semihosting; that is a later
transition (protocol.md §5.5).

## Required changes (confirmed by the firmware team, 2026-10-22)

| # | Change | protocol.md |
|---|---|---|
| F1 | **Port 59 FW update**: same commands as port 3 (`set-time`, `fw-start`, `fw-block`, `fw-schedule`); payload = one CBOR map, **no inner COBS, no CRC**. Keep port 3 until all hosts are migrated | §5.4 |
| F2 | **Semihost time ops** (added to the legacy op set, not ARM semihosting): `0x10` SYS_CLOCK → `u32` centiseconds; `0x11` SYS_TIME → `i64` seconds; `0x30` SYS_ELAPSED → `u64` ticks; `0x31` SYS_TICKFREQ → `u64` ticks/s. No arguments. The reply is the bare value: no `r`/`errno` prefix | §5.5.3 |
| F3 | Accept a 31-bit fid in `WRITE` (`value & 0x7FFFFFFF`), bit 31 = no reply | §5.5.2 |
| F4 | Expect **newlib errno** values in semihost replies | §5.5.4 |
| F5 | Accept `-1, ENOSYS` for ops the host does not implement (SYSTEM, RENAME, UNLINK, LINK, STAT, LSEEK, unknown). Legacy hosts never replied, so this path was never exercised | §5.5.4 |
| F6 | **Frame size limit** (requested by the firmware team, 2026-10-23): `LOG_MAX_PACKET_SIZE` 1500 → **1443**, so every frame is at most 1452 bytes on the wire (one unfragmented UDP datagram on a 1500-byte MTU with IPv6). The semihost `WRITE` data is at most 1438 bytes per request and `READ` replies at most 1024; FW update `block-size` ≤ 1024 | §2.4 |

## Confirmed by the firmware team (2026-10-22)

| # | Behaviour | protocol.md |
|---|---|---|
| C1 | Error replies with `r = -1` (incl. READ and WRITE, where legacy hosts returned `0`) are handled | §5.5.4 |
| C2 | The **FSTAT** (op 6) error reply keeps its full 48 bytes (`-1, errno`, then zeros) | §5.5.4 |
| C3 | A malformed request gets `-1, EINVAL` | §5.5.4 |
| C4 | The device sends the **port-62 device info at boot** | §5.2 |
| C5 | Every frame the device sends starts with `00`; idle pulses at least every 0.5 s | §2.1, §2.3 |
| C6 | UART RX powers down after 2 s without received bytes and wakes on an edge. **Hosts wait ≥ 50 ms** after the wake pulse, because USB-UART converters buffer before the edge reaches the target | §2.3 |
| C7 | The HID report descriptor uses a vendor-defined usage page | §4.3 |
| C8 | `fw-schedule` replies with an `error` key when Ed25519 verification fails | §5.4 |
| C9 | **TIMEOFDAY** (op 13) stays as is (`u64 tv_sec, u32 tv_usec`), with the same clock as SYS_TIME; the hub answers it like the other time ops | §5.5.3 |
| C10 | The STLINK `len % 8 == 0` workaround stays for now | §4.1 |

## Not changed

Framing (COBS), the frame header, log frame layout, ports 0/1/60/61/62, BLE UUIDs, HID report IDs
(`0xA1` in, `0xA2` out) and the `.logdata` format are unchanged.
