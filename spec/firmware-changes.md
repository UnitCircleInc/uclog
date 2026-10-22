# Firmware changes for protocol v1 (Phase 0)

The firmware is not in this repository. This is the list of changes the firmware team needs to
make, or confirm, so that it matches [protocol.md](protocol.md). Hosts are built against these
behaviours, and the vectors in [vectors/](vectors/README.md) can be used in firmware unit tests.

## Required changes

| # | Change | protocol.md |
|---|---|---|
| F1 | **Port 59 FW update**: same commands as port 3 (`set-time`, `fw-start`, `fw-block`, `fw-schedule`); payload = one CBOR map, **no inner COBS, no CRC**. Keep port 3 until all hosts are migrated | §5.4 |
| F2 | **Semihost time ops**: `0x10` SYS_CLOCK → `u32` centiseconds; `0x11` SYS_TIME → `i64` seconds; `0x30` SYS_ELAPSED → `u64` ticks; `0x31` SYS_TICKFREQ → `u64` ticks/s. No arguments. The reply is the bare value: no `r`/`errno` prefix | §5.5.3 |
| F3 | Accept a 31-bit fid in `WRITE` (`value & 0x7FFFFFFF`), bit 31 = no reply | §5.5.2 |
| F4 | Expect **newlib errno** values in semihost replies | §5.5.4 |
| F5 | Accept `-1, ENOSYS` for ops the host does not implement (SYSTEM, RENAME, UNLINK, LINK, STAT, LSEEK, unknown). Legacy hosts never replied, so this path was never exercised | §5.5.4 |

## Please confirm (hosts assume "yes")

| # | Assumption | protocol.md |
|---|---|---|
| C1 | Error replies for **READ** and **WRITE** now return `r = -1` (legacy hosts returned `0`). Is `-1` handled? | §5.5.4 |
| C2 | The **FSTAT** error reply is padded to its full 48 bytes (`-1, errno`, zeros). Is that what the firmware reads? | §5.5.4 |
| C3 | A malformed request gets `-1, EINVAL` | §5.5.4 |
| C4 | The device sends the **port-62 device info at boot** (hosts use it to detect a reset or image change) | §5.2 |
| C5 | Every frame the device sends starts with `00`, and the device sends idle pulses at least every 0.5 s | §2.1, §2.3 |
| C6 | UART RX power-down after 2 s without received bytes, wake on edge; a ≥ 2 ms host pause after the wake pulse is enough | §2.3 |
| C7 | The HID report descriptor uses a **vendor-defined usage page** (`0xFF00`–`0xFFFF`); WebHID refuses other pages | §4.3 |
| C8 | `fw-schedule` replies with an `error` key when Ed25519 verification of the received image fails | §5.4 |
| C9 | `TIMEOFDAY` (13) stays as is (`u64 tv_sec, u32 tv_usec`), with the same clock as SYS_TIME | §5.5.3 |
| C10 | The STLINK `len % 8 == 0` workaround is still needed with current STLINK firmware (test with it disabled) | §4.1 |

## Not changed

Framing (COBS), the frame header, log frame layout, ports 0/1/60/61/62, BLE UUIDs, HID report IDs
(`0xA1` in, `0xA2` out) and the `.logdata` format are unchanged.
