# Plan 08 — Public repository, distribution and documentation site

Shared context, protocol and decisions: [README.md](README.md). D# = decisions (README §1).

## 1. Goal

Make ucLog public as one product: the device library and the host tools, with published build
products and a documentation site at **uclog.unitcircle.ca**.

| Product | Published as |
|---|---|
| Python library | PyPI (`uclog`, plan 07 tasks 11–12); `python/` in the repository, with `python/examples/` |
| Web app | uclog.unitcircle.ca/app/ (GitHub Pages), usable offline once loaded; a single-file download (`uclog-app-N.html`) on the site and each release (§3.1) |
| Native `uclog` / `uclog-hub` | GitHub Releases (plan 07). The site has a download page that links to the release assets: Pages cannot run them, and release assets have no 100 MB file limit |
| Embeddable `<uc-log>` | npm `@uclog/element` (plan 07 task 13a); versioned single-file builds on the site (uclog.unitcircle.ca/element/N/ and `/element/latest/`); `web/` in the repository, with the reference integration |
| VS Code extension | VS Code Marketplace and Open VSX (plan 07 task 13b) |
| Documentation | uclog.unitcircle.ca, built with Zensical and the www.unitcircle.ca theme; one copy per release with a version selector (§3.2) |
| Device library | in the repository (§2), plus a small clone for device-only users (§2.3) |

Plan 07 (remote, CI, signing, publishing jobs, first release) comes first. This plan adds the
repository layout, the site and its content.

## 2. One repository or two

Today:
- [UnitCircleInc/uclog](https://github.com/UnitCircleInc/uclog) (public) is the device side as a
  Zephyr module: `include/log.h`, `lib/log.c`, `lib/logserver.c`, the `ucuart`, `ucuart_z` and
  `ucusb` drivers, Kconfig, snippets, the `apphash`/`signature` CMake helpers, and the legacy
  Python host scripts (`scripts/uclog.py`, `logdata.py`, `sbl.py`, …). 21 commits. Its sources
  carry Apache-2.0 headers, but it has no `LICENSE` file.
- It depends on [UnitCircleInc/ucComm](https://github.com/UnitCircleInc/ucComm) (COBS, CBOR,
  CRC-32C, `cb`), another Zephyr module.
- This repository (host side, Apache-2.0) has its own bare-metal copy of the device library in
  `firmware/uclog/`, with the protocol changes of `spec/firmware-changes.md` (1443-byte frames, the
  F7 device-info request, RX rejection rules). Its `firmware/uccomm/` is a copy of ucComm at
  `8bc8e3b`. The two `log.c` files have already diverged.

### 2.1 Tradeoffs

| | One repository (device library moved in) | Two repositories (as now) |
|---|---|---|
| Protocol changes | One commit changes the spec, the device library, every host and the test vectors. CI runs the device encoder against `spec/vectors` and the hosts against the device | Two pull requests that must land together; the device side can't be tested against the hosts in CI |
| Duplicate `log.c` | Gone: one library with a Zephyr port and a bare-metal port; the test boards build it | Stays: the bare-metal copy here, and the Zephyr one there, kept in step by hand |
| Hardware tests | The test firmware and the hosts it exercises are in one place (already true for the bare-metal copy) | The Zephyr library has no hardware tests next to it |
| Issues, docs, releases | One tracker, one site, one changelog; the compatibility table (device ↔ host versions) is in the same place as both | Spread out; users report host problems against the device repository and vice versa |
| Device-only users | Pull a much larger repository (Rust, TypeScript, Python, docs) unless there is a small clone (§2.3) | Small clone with only the device code |
| Zephyr `west` | Works: a `zephyr/module.yml` at the root points `cmake`/`kconfig` into the device directory. `west update` fetches the whole repository; it has no sparse checkout | Works as now |
| Versioning | Device library and hosts need separate version lines in one repository (separate tag prefixes) | Natural: one version line each |
| CI | Path filters so device-only changes don't run the Rust/web/VS Code jobs, and the other way round | Separate and simpler |
| Existing users | Their `west.yml` URL changes, unless the monorepo takes over the `uclog` name or a mirror keeps it (§2.2) | Nothing changes |
| Legacy scripts | `scripts/uclog.py` and friends (frozen, D27) are retired in the move; `logdata.py`, `cachelogdata.py`, `apphash`, `signature` stay as build-time tools | Stay where they are |

**Recommendation: one repository, with a published device-only mirror.** The protocol is shared,
and keeping two `log.c` files in step is already a real cost. The main drawback, device users
pulling everything, is solved by a mirror (§2.3).

### 2.2 Naming (decided 2026-10-25: option A)

- **Monorepo: `UnitCircleInc/uclog`**. This repository is pushed there, and the current device
  history is imported under `device/`. Everything sits under the name people know, which matches
  the site.
- **Device-only clone: `UnitCircleInc/uclog-device`**, a read-only mirror.
- **Existing `west.yml` URLs keep working** through a root `zephyr/module.yml`, but they fetch the
  whole monorepo. The docs point Zephyr users at `uclog-device` for a small clone.

Rejected:
- Option B: a new `uclog-client` repository, with `uclog` becoming the mirror.
- Option C: keeping two repositories.

### 2.2a Versions: SoloVer (decided 2026-10-25)

[SoloVer](https://beza1e1.tuxen.de/SoloVer): one non-negative integer, optionally followed by a
postfix. A postfix starts with `-` (a pre-release, sorts before the number) or `+` (a hotfix,
sorts after it). Postfixes compare alphanumerically: `1-beta1 < 1 < 1+hotfix1 < 2`.

- **Two version lines.** The host tools are tagged `vN` (Rust, Python, VSIX, `<uc-log>` and the
  web app in step). The device library is tagged `device-vN`.
- **Numbering starts at 4** (decided 2026-10-25). The device repository already has tags `v1`, `v2`
  and `v3`; they come along with the imported history and still point at the old layout, so
  current users who pin them (`west.yml` `revision: v3`, a submodule) are unaffected. The first
  release of the new layout is tagged `v4` and `device-v4` on the same commit; after that the two
  lines move independently. Nothing below 4 is ever published from the monorepo, so `v1`–`v3`
  always mean the old device-only layout. The mirror (§2.3) carries the device line as plain
  `vN` tags (`v1`–`v3` from the old history, then `v4` from `device-v4`), so device-only users
  see one unbroken series.
- **Protocol version.** The wire protocol keeps its own number (protocol.md, v1). The
  compatibility table maps device versions to host versions through it.
- **Postfixes** are limited to `-alphaK`, `-betaK`, `-rcK` and `+hotfixK`, with K from 1 to 9.
  Plain alphanumeric order puts `alpha10` before `alpha2`, so a tenth one starts the next letter or
  number.
- **Each ecosystem gets a mapped number.** Their rules require it, so a CI step derives every
  manifest's version from the tag and checks it (plan 07 task 13):

  | SoloVer tag | PyPI (PEP 440) | Cargo, npm (SemVer) | VS Code Marketplace / Open VSX |
  |---|---|---|---|
  | `N` | `N` | `N.0.0` | `N.0.0` |
  | `N-alphaK`, `N-betaK`, `N-rcK` | `NaK`, `NbK`, `NrcK` | `N.0.0-alpha.K`, `-beta.K`, `-rc.K` | not published: the VSIX goes to the GitHub release only (the Marketplace allows no pre-release suffix) |
  | `N+hotfixK` | `N.postK` | `N.0.K` | `N.0.K` |

  - **PyPI** refuses `+` local versions, so hotfixes become post-releases.
  - **SemVer** ignores `+` build metadata when ordering, so hotfixes go in the patch number.
  - **The device library** reports its SoloVer string as is (`UCLOG_DEVICE_VERSION` in `log.h`,
    and in the device info).

### 2.3 Cloning only what you need

- **Device-only mirror** (`uclog-device`, §2.2): CI runs `git subtree split
  --prefix=device` on every push to `main` and on each device tag, and pushes the result. It is a
  normal small repository with its own history, so it works with `west`, CMake `FetchContent`,
  git submodules and plain clones. Its README says where pull requests go.
- **Release archives**: each device tag attaches `uclog-device-N.tar.gz` (the library, ucComm
  and the build helpers), for Makefile projects with no git dependency.
- **Partial clone of the monorepo** (documented for people who want one directory):
  ```sh
  git clone --filter=blob:none --sparse https://github.com/UnitCircleInc/uclog
  cd uclog && git sparse-checkout set device        # or python, web/examples/embed, …
  ```
- **ucComm**: keep it a separate repository; it is shared with other projects (for example
  `ucsbl`). The device library pins a version: a `west.yml` entry for Zephyr, a pinned copy in the
  release archive and the mirror for Makefile and CMake users.
- Python users install from PyPI. The examples are under `python/examples/`, which a sparse
  checkout can fetch alone.

### 2.4 Layout after the move

```
device/            the device library (one log.c): include/, src/, ports/{zephyr,baremetal}/,
                   drivers/ (ucuart, ucuart_z, ucusb), dts/, snippets/, Kconfig, CMakeLists.txt,
                   zephyr/module.yml, cmake/ (log, apphash, signature), tools/ (logdata.py,
                   hash.py, …), uclog.mk, examples/ (bare metal: make and CMake; Zephyr)
firmware/          test firmware for the hosts (boards + app, and zephyr/), built against device/
native/ python/ web/ spec/    as now
website/           the documentation site (Zensical): zensical.toml, docs/, overrides/
docs/plans/        design plans (not published)
zephyr/module.yml  points into device/
```

### 2.5 One device library (task 4; decided 2026-10-26)

The bare-metal library in this repository is the base. It already implements the spec (F1–F7,
host presence, the RX rules). The Zephyr pieces are ported onto it.

- **Core** `device/src/log.c` + `device/include/log.h`: framing, logs, device info (with
  `uclog_version`), host presence (2 s without RX, 0.5 s pulses, device info on return), the RX
  rules, `log_tx`/`log_rx`/`log_notify`. The Zephyr options that are not Zephyr-specific move here:
  the saved log across a reset (`UC_LOG_SAVE`) and the signed-image app hash (`UC_SIGNED_IMAGE`,
  `sbl_app_hash()`). The C++ `typechar` templates come from the Zephyr header.
- **Port interface** `device/include/log_port.h` (`log_port_*`, as in `firmware/uclog/log.h`
  today), plus weak hooks for how `log_rx` waits: a polled port runs `log_poll()`, a threaded port
  blocks the caller until the dispatcher has the reply.
- **Bare metal**: the board implements `log_port_*`; the main loop calls `log_poll()` and a 1 kHz
  tick calls `log_tick()` (unchanged).
- **Zephyr** `device/ports/zephyr/`: `log_port_*` over the `ucuart` driver API; a log thread runs
  `log_poll()`/`log_tick()` (woken by RX, else every 10 ms); `SYS_INIT` start-up as before; the
  `LOG_ERR`… mapping (`CONFIG_LOG_CUSTOM_HEADER`), printk/stdout hooks, the shell backend
  (`UC_SHELL`). Zephyr 3.7 (NCS 2.9) and 4.x.
- **Host presence is the core's**. The `ucuart` API gains `tx_pause`, `tx_idle` and `rx_mark`; the
  drivers stop gating TX on their own `host_ready` timers. `ucuart_z` keeps its timer for power
  management only (at 2 s); `ucusb` keeps its DTR gating. The nRF drivers (`ucuart`, `ucusb`) are
  build-tested only (no nRF hardware here); `ucuart_z` is tested on the NUCLEO boards.
- **Semihost** `device/src/syscalls.c` (issue #2): the syscalls from `firmware/app/sh/syscalls.c`,
  for newlib and picolibc; Zephyr `CONFIG_UC_SYSCALLS`.
- **Tools** `device/tools/` (was `scripts/`): `logdata.py` is the standard-library version from
  `firmware/uclog/tools` (accepts `--ofile` too); `hash.py`, `cachelogdata.py` (the hosts' cache
  directory rules), `sbl.py` and its modules. CMake helpers and linker fragments in `device/cmake/`.
- **Test firmware** builds `device/`; `firmware/uclog/` is removed. A Zephyr build of the same test
  app (`firmware/zephyr/`, NUCLEO-L452RE-P over `ucuart_z`) runs the hardware tests.
- **Dropped**: `UC_FAULT` (no source), `lib/logserver.c` and `lib/junk.c` (folded into the core and
  the Zephyr port).

## 3. The site (uclog.unitcircle.ca)

- **Hosting**: GitHub Pages from Actions on this repository. The DNS record is a CNAME from
  `uclog.unitcircle.ca` to `unitcircleinc.github.io`, and HTTPS is enforced. Pages needs the
  repository to be public, or a paid plan while it is private.
- **One deploy job** builds three things and uploads them as one Pages artifact:
  - the Zensical site at `/`;
  - the web app (`web/apps/demo`) at `/app/`;
  - the element builds at `/element/N/` and `/element/latest/`.

  Older element versions and docs versions (§3.2) must stay published. Each deploy fetches them
  from the release assets, because a Pages deploy replaces the whole site.
- **Theme**: copied from [uc-www](https://github.com/UnitCircleInc/uc-www) (`zensical.toml` theme
  section, `overrides/`, `stylesheets/extra.css`, logo and favicons). Record the commit it came
  from, and update by hand. Built with `uv run zensical build` like uc-www.
- **Single sources**:
  - The protocol pages include `spec/protocol.md` (pymdownx snippets), so the spec is not copied.
  - The device code snippets come from files that CI compiles.
  - The Python examples are files that CI runs.
- **The web app on a public HTTPS origin**:
  - Web Serial, WebHID and Web Bluetooth all work there.
  - **The hub's default `--allow-origin` list must add `https://uclog.unitcircle.ca`**. Today it
    holds only `localhost`, `127.0.0.1` and VS Code webviews, so the app's hub connections would
    otherwise need approval each time.
  - Chrome's Local Network Access permission prompt (public site → `ws://localhost`) has to be
    checked and documented.
  - The logdata service ([fw.unitcircle.ca](https://fw.unitcircle.ca/docs/errors-and-limits)):
    - Its customer API (`/logdata`) accepts the account's own `https://{domain}` and its
      subdomains, origins registered for the account, and origins the operator allows. Token-free
      log reads, when an account enables them, accept any origin.
    - So `https://uclog.unitcircle.ca` must be on the operator's allow list, or else customers on
      other domains can't fetch logdata from the hosted app with a token.

### 3.1 The web app offline and as a single file (decided 2026-10-25)

- **Offline at `/app/`**: a service worker precaches the app shell, workers and wasm (Vite PWA
  plugin, or a small hand-written worker). After one visit the app loads with no network. Updates
  apply on the next load, with a "new version" notice; the worker's cache is keyed by version.
- **Single-file download**: a second build inlines everything (scripts, CSS, workers as blob URLs,
  wasm as base64) into one `uclog-app-N.html`. It is linked from the app ("Download for offline
  use"), the download page, and each GitHub release. It opens from disk with no server, for
  machines with no network or a locked-down browser profile. The element's single-file build
  (§1) is the starting point.
- **To check for `file://`**:
  - Web Serial, WebHID and Web Bluetooth in a `file://` page (Chrome treats `file://` as a secure
    context; each API's own rule has to be checked). If one is refused, the download page says
    which links work.
  - Hub connections: the page's Origin is `null`. The hub refuses it by default, so the docs
    show `uclog hub --allow-origin null` (an explicit opt-in; plan 01 §4.3), and the app explains
    the refusal when it sees it.
  - Storage (settings, logdata cache): `file://` pages share one origin per browser, so keys are
    prefixed.
  - Logdata service fetches send `Origin: null`, which the service refuses for token requests.
    Offline use gets logdata from the ELF, `.logdata` files or the cache instead; the docs say so.
    Making them work is a future task (task 20).

### 3.2 Versioned docs (decided 2026-10-25)

- **One copy per release** at `/vN/`, plus `/latest/` (the newest release; `/` redirects to it)
  and `/dev/` (built from `main`). The theme's version selector lists them, newest first, with
  pre-releases marked.
- **How** (decided 2026-10-25): mike's layout and `versions.json`, which the Material-derived
  version selector reads. Use mike itself if it works with Zensical's build; otherwise build each
  version ourselves and write `versions.json` in mike's format.
- **Keeping old versions**: each release attaches its built site (`uclog-docs-N.tar.gz`). The Pages
  deploy unpacks every one into `/vN/`, then builds `latest` and `dev`. A broken old version is
  fixed by a `+hotfixK` release, not by editing the archive.
- **Device and host versions**: the docs follow the host line (`vN`). Each version's device pages
  name the device version they describe, and the compatibility table links the two.
- **Search** covers the version being viewed. Pages outside the docs (`/app/`, `/element/…`, the
  download page) are not versioned; the download page lists every release.

## 4. Documentation content

| Section | Contents |
|---|---|
| Getting started | What ucLog is; install each host (pip, VS Code, download, the web app link); first connection with the simulated device, then a board |
| Using the hosts | With screenshots: the web app, VS Code, `uclog gui`, `uclog tui`, the CLI (`list`, `decode`, `term`, `fw-update`, `hub`), Python. Connecting over each link (serial, CDC, HID, BLE, UDP, a remote hub), the terminal and command buttons, plots, device info, FW update, semihost files |
| From device code to screen | Side by side: device C code and what each host shows. Levels, 0–4 arguments, ints, floats, strings, `{enum:name}%d`, `{sym}%p`, `LOG_MEM_*`, terminal commands (port 61), plots (port 60), semihost `printf`/files, FW update |
| Adding ucLog to a device project | The port interface; UART, USB CDC/HID, BLE and UDP links; **Makefile** (`uclog.mk`), **CMake** (`add_subdirectory` / `FetchContent`), **Zephyr** (west module, Kconfig, snippets); generating logdata (`.logdata_cbor` in the ELF, app hash); minimal examples simplified from `firmware/` (one bare-metal, one Zephyr) |
| Logdata and services | How hosts find logdata: the ELF, `.logdata` files, the cache in `~/.cache/uclog` (what is stored, keyed by app hash, how to clear it), offline use. The logdata service when configured: what ucLog sends (`GET /logdata/{domain}/{id}` with the bearer token, then the 5-minute download link it redirects to), where each host keeps the token, and uploading logdata at build time. The service itself is documented publicly at [fw.unitcircle.ca/docs](https://fw.unitcircle.ca/docs) (accounts, tokens, API, CORS, limits); link to it rather than repeat it |
| AI assistants | The service's MCP server ([fw.unitcircle.ca/docs/mcp](https://fw.unitcircle.ca/docs/mcp), `https://fw.unitcircle.ca/mcp`, bearer token): an assistant can list firmware and logdata, fetch download links, and check what a device would be offered. Setup for Claude Code, Cursor and VS Code, linked from the service docs |
| Embedding `<uc-log>` | Script tag vs npm; attributes, events, the host services (`rpc.ts`), styling (custom properties), workers (`worker-base`), CSP, self-hosting. The worked example is `web/apps/demo`, the site's own web app (§5) |
| Troubleshooting | Per platform, from what testing found: Windows (Restricted Mode turns the VS Code extension off; current Bluetooth drivers for BLE speed, ~28 vs ~82 kB/s; `setx` reaches new windows only; direnv not recommended), WSL2 (no USB/BLE, use a hub; no keyring; cursor theme; WSLg restarts), Linux udev rules, macOS Bluetooth permission. Links to firmware/README.md and native/README.md |
| Security | Hub admission, the Origin check (D14; plan 01 §4.3), TLS, the semihost sandbox, logdata tokens |
| Protocol and performance | The rendered spec; transports; performance (§6) |
| Reference | CLI options, Python API (generated from docstrings), `<uc-log>` API (generated from the TypeScript), device API (`log.h`) |
| Release notes and compatibility | Changelog; a table of device library versions against host versions |

**Screenshots** are generated, not taken by hand, so they stay current:
- the web app and VS Code with Playwright (`web/e2e/screens.spec.ts` already does some);
- the GUI from its headless UI tests (egui snapshots);
- the TUI from ratatui's `TestBackend`, rendered to SVG.

## 5. Embedding reference

- `web/apps/demo` (the site's web app) is already `<uc-log>` plus ~30 lines of `main.ts`. Make it
  the documented reference: comments, no private imports, built only from the published package.
- Merge `web/examples/embed` (a branded portal, events, host services) into the docs as a second,
  fuller example.
- Both are tested in CI (Playwright, `embed.spec.ts` exists), so the docs' example cannot rot.

## 6. Performance and footprint

A `bench` command in the test firmware, and a script that writes the results as JSON. The docs
render their tables from that JSON, so they can be regenerated per release. Boards: C071
(Cortex-M0+), L452 (Cortex-M4F), WBA55 (Cortex-M33).

| Metric | ucLog | Compared with |
|---|---|---|
| Wire bytes per log statement, 0–4 arguments (int, float, string), including COBS and the frame header | measured | the same message formatted as text by `snprintf` |
| Encode time per statement (cycles; DWT CYCCNT on the M4/M33, SysTick on the M0+), per argument kind | measured | newlib-nano `snprintf` (with float support) of the same message |
| End-to-end throughput per link (logs/s, B/s) | from `burst` (UART 100 kB/s, CDC ~350 kB/s, HID 63 kB/s, BLE ~72 kB/s; firmware/README.md) | a text log over the same link |
| Footprint: flash and RAM of the library, ucComm and buffers (configurable) | `arm-none-eabi-size` per object, `-Os` | newlib-nano printf with float support |
| Host side: decode rate and hub latency | measured | — |

## 7. Tasks

| # | Task | Size |
|---|---|---|
| 1 | Versions (§2.2a, starting at 4): the tag → manifest mapping and its CI check (Cargo, `pyproject.toml`, `package.json` files, VSIX), `UCLOG_DEVICE_VERSION` and its place in the device info, the compatibility table. **Done 2026-10-26**: `scripts/version.py` (`show`, `check TAG`, `set N [--device]`, `map N`) with tests; CI runs `show` and the tests (`spec.yml`) and `check` on a release tag (`release.yml`, before the builds). The tree is at `4-alpha1` (Cargo/npm/VSIX `4.0.0-alpha.1`, PyPI `4a1`); the native binaries show the SoloVer form (`uclog 4-alpha1`, `UCLOG_VERSION` from `build.rs`). `UCLOG_DEVICE_VERSION` in `firmware/uclog/log.h` (moves to `device/` in task 4) is sent as `uclog_version` in the device info (protocol.md §5.2). The table is `spec/compatibility.md`: device `v1`–`v3` are not supported by v4 hosts (their raw app hash on port 63 collides with semihost) | S |
| 2 | Add a `LICENSE` file to the device repository now (Apache-2.0, as its headers say); check the third-party notices (ST HAL/CMSIS are not redistributed: the test firmware points at STM32Cube packages). **Done 2026-10-26** (device repo `0970e82`): no third-party code needs a notice; `ucuart_z.c` is StarIC's contribution under Apache-2.0 | S |
| 3 | Import the device repository with its history under `device/` (`git subtree add`, or `git filter-repo --to-subdirectory-filter`), and retire `scripts/uclog.py` and the other legacy host scripts (D27). Import `main`, not `v3`: it has StarIC's STM32N6 changes after `v3` (PR #3: the device-info CBOR packet in place of the raw hash, `apphash.cmake` and log caching compatible with the STM32 signing tool, the `ucuart_z` driver on the Zephyr UART API, `cb.c`/`cbor.c` removed in favour of ucComm). **Done 2026-10-26**: `git subtree add --prefix=device` of `main` at `0970e82` (no squash, so `v1`–`v3` are the device repository's own commits; the tags were fetched). Retired `scripts/uclog.py`, `uccon.py` and `cobs.py` (only `uclog.py` used it); the build-time tools stay, listed in `device/scripts/README.md`. Later device-repository commits come in with `git subtree pull --prefix=device` until the repository is pushed (task 19) | S |
| 4 | One `log.c`: merge the bare-metal library (`firmware/uclog`) with the Zephyr one. Port interface; the Zephyr `ucuart`/`ucuart_z`/`ucusb` drivers move to it; apply `spec/firmware-changes.md` (F1–F7) to the Zephyr side; the test boards build `device/`. Keep the post-`v3` changes (task 3) and reconcile their device-info packet with F7. **Fixes device issue #2 "Fix support for semihosting"**: the Zephyr side has no port-63 semihost client, only stdout/printk hooks into logs (`lib/log.c`, TODOs: stdin not hooked, a possible picolibc conflict). Move `firmware/app/sh/syscalls.c` into the library and hook it into Zephyr's libc (picolibc and newlib) for stdin/stdout and files. Goal (confirmed 2026-10-26): the device library reaches functional parity with this repository's target code (`firmware/uclog`, `firmware/app/sh`); after that the test firmware here uses `device/` and keeps no copy. Close the issue when parity is reached. **Done 2026-10-26** (§2.5): one `device/src/log.c` (the bare-metal library, so F1–F7 hold on Zephyr too) with `log_port.h` and thread hooks; the Zephyr port `device/ports/zephyr/` (log thread, `SYS_INIT`, `LOG_*` mapping, shell, `UC_LOG_SAVE`, `UC_SIGNED_IMAGE`; `log_init` is `uclog_init` on Zephyr, whose logging has a `log_init`); host presence moved out of `ucuart`/`ucuart_z`/`ucusb` (`tx_pause`, `tx_idle`; `ucusb` no longer builds its own device info); semihost as `device/src/semihost.c` + `syscalls.c` (newlib) + `ports/zephyr/semihost_fs.c` (a Zephyr file system at `/host`, stdout/stdin hooks); `UC_FAULT` dropped. Fixed on the way: `ucuart_z` skipped log bytes after each device-info prefix (async TX), did not select `EVENTS`, and had a `const` error; `ucusb` did not select `HWINFO`; `ucuart` builds with nrfx 4 (Zephyr 4.x). The boards here build `device/` (`firmware/uclog/` is gone) and pass the hardware tests (L452: Chrome and VS Code; C071: UART, CDC, HID). `firmware/zephyr/` runs the test app on Zephyr 4.3 on the L452 through `ucuart_z` and passes both hardware tests, semihost included (it needs circular RX DMA: firmware/README.md). Build-tested: Zephyr 3.7 (NCS 2.9) and 4.3, nRF52840 with `ucuart`, `ucusb` and `ucuart_z`, with `UC_SYSCALLS`, `UC_LOG_SAVE` and `UC_SHELL`. Not tested: the nRF drivers on hardware, `UC_SIGNED_IMAGE` (needs the sbl library), `UC_LOG_SAVE` across a real reset. Issue #2 stays open until this is pushed | L |
| 5 | Build integrations: `zephyr/module.yml` at the root, `device/CMakeLists.txt` usable outside Zephyr, `device/uclog.mk` for Makefiles; ucComm pinned for each. **Done 2026-10-26**: ucComm v3 (`8bc8e3b`) is vendored once in `device/uccomm` (was `firmware/uccomm`); `device/west.yml` pins the same tag, and on Zephyr the copy is used when the workspace has no ucComm module (`select UCCOMM if ZEPHYR_UCCOMM_MODULE`). The root `zephyr/module.yml` (module `uclog`) points into `device/`. `device/CMakeLists.txt` is the Zephyr module or a plain CMake library (`uclog`, `uclog_syscalls`, `uclog_add_logdata()`); `device/uclog.mk` (`UCLOG_SRCS`, `UCLOG_CFLAGS`, `uclog_post_link`) builds the three boards, whose logdata is unchanged. Both checked: the L452 firmware through the plain CMake path gives the same logdata; Zephyr builds with the root module and with `device/` + the ucComm module. ucComm v3 did not build on x86 (a `__x86_64_` typo, and a `va_list` copied into a struct); fixed upstream in v5 (`a00bdc4`); v6 (`beb20cf`, 2026-10-27) also fixes its float writers on x86, and is the one vendored and pinned | M |
| 6 | Device CI: build the examples and the test firmware (arm-none-eabi, a Zephyr board in the Zephyr SDK container); a host build of the encoder run against `spec/vectors`. **Done 2026-10-26**: `.github/workflows/device.yml`. `host` (x86-64 and Arm64): `device/test/host`, the library with an in-memory port, checked by `test_vectors.py` against `spec/vectors` and `ucref` (port frames and the COBS boundaries, the `peek_mux` port cases, every log argument kind, the device info and F7, the scanner vectors, pulses and host presence). `firmware`: the three boards with Ubuntu's Arm GCC and the STM32Cube parts fetched from ST's GitHub (`.github/scripts/fetch-stm32cube.sh`). `zephyr`: Zephyr v3.7.2 and v4.4.2 (each in the CI image with its SDK; manifests in `.github/zephyr/`), `.github/scripts/zephyr-builds.sh`: the sample on the L452 and on the nRF52840-DK with each driver and every option, and `firmware/zephyr` (4.x). All run locally the same way (the host test in Docker, the firmware job in `ubuntu:24.04`, the Zephyr builds with SDK 0.16.9 and 1.0.1); not yet on GitHub. Fixed on the way: Zephyr 3.7 declares no printk/stdout hook functions; nrfx 4 (Zephyr 4.4) has no `nrfx_err_t` (`ucusb`), and `ucusb`'s init check never fired with nrfx 3. The 4.4.2 build passes the hardware tests on the L452 | M |
| 7 | Device-only mirror `UnitCircleInc/uclog-device` (§2.3): the `subtree split` job, its README, the `uclog-device-N.tar.gz` release archive, and its `vN` tags (`v1`–`v3` from the old history, then from `device-vN`). **Done 2026-10-26** (not yet run): `.github/workflows/device-mirror.yml` pushes `git subtree split --prefix=device` to the mirror's `main` on every push to `main` (never forced: the split is the same each time; it keeps the old repository's commits, so `v1`–`v3` are pushed as they are), and on `device-vN` checks `UCLOG_DEVICE_VERSION`, pushes `vN`, and attaches `uclog-device-N.tar.gz` (+ `.sha256`) to the draft release. `device/README.md` is the mirror's README (absolute links out of `device/`). Checked locally against a bare repository. Needs, at task 19: the `UnitCircleInc/uclog-device` repository and a write deploy key whose private half is the secret `UCLOG_DEVICE_DEPLOY_KEY` | S |
| 8 | A small device plot API (`log_plot_create`, `log_plot_data`, `log_plot_clear` over port 60, protocol.md §5.6): today plots need hand-built CBOR, which is hard to document. **Done 2026-10-27**: `device/include/log_plot.h` + `src/log_plot.c` (line plots: up to 3 axes of 9 series with labels, limits and props, `xscroll`; `log_plot_point` for one point per series, `log_plot_data` for n points of one series, `log_plot_clear`), built on ucComm's CBOR writer in a stack buffer (`LOG_PLOT_BUF_SIZE`, 256). Checked by the host test (`Plots`, the decoded maps) and on hardware: the test firmware's `plot` command draws in Chrome (`web/e2e/hw.spec.ts`) on the L452 and the C071. ucComm v5 wrote whole-number floats wrongly on x86 (its `float16_t` is an `int16_t` there); fixed in v6 | S |
| 9 | `website/`: Zensical project, theme from uc-www, spec included from `spec/`; Pages deploy job (site + `/app/` + `/element/…`); custom domain and HTTPS. **Done 2026-10-27**, except the Pages settings and DNS (task 19): `website/` (Zensical 0.0.73 via uv; the "Sumi & Ai" theme copied from uc-www `fe4e402`, see `website/THEME.md`); pages Home, Getting started, The hosts, Web app, and the protocol (`spec/protocol.md`, `spec/compatibility.md`, generated by `website/build.py docs`: repository links point at GitHub at the version's ref, lists normalised for Python-Markdown); strict build. `.github/workflows/pages.yml` builds on pull requests and deploys on `main`, releases and by hand (only in `UnitCircleInc/uclog`): docs, `/app/`, `/element/` and `CNAME` | M |
| 9a | Versioned docs (§3.2, mike): `/vN/`, `/latest/`, `/dev/`, the version selector, `uclog-docs-N.tar.gz` per release and their reassembly in the deploy. **Done 2026-10-27**: mike's layout without mike (it drives `mkdocs`, not Zensical): Zensical's selector (`extra.version.provider = "mike"`) reads `versions.json`; `website/build.py assemble` writes `/vN/`, `/dev/`, `/latest/` (a copy of the newest release that is not a pre-release; SoloVer order from `scripts/version.py`), `versions.json`, the `/` redirect and `404.html`. The release workflow's `site` job attaches `uclog-docs-N.tar.gz`, `uclog-app-N.html` and `uclog-element-N.iife.js`; the deploy downloads every release's and reassembles. Checked by assembling with and without fake releases and switching versions in Chrome | M |
| 9b | Web app offline (§3.1): service worker at `/app/`; the single-file `uclog-app-N.html` build, its download link and release asset; the `file://` checks, and `--allow-origin null` documented. **Done 2026-10-27**: the production build writes `sw.js` (a cache per build, cache-first, old caches deleted on activate); a new build shows **Reload**. `vite build --mode single` makes `uclog-app.html` (IIFE, workers and the sample logdata inlined, no app bar); the hosted app links it ("Download for offline use"). From `file://` in Chrome 156 the page is a secure context with Web Serial, WebHID and Web Bluetooth; its origin is `null`, so a hub needs `--allow-origin null` (the app says so on the 1008). Module workers from `blob:` fail in a `file://` page, so `<uc-log>`'s inlined workers are now classic. `web/e2e/offline.spec.ts`; documented in `website/docs/web-app.md` | M |
| 10 | Hub: add `https://uclog.unitcircle.ca` to the default `--allow-origin`; check Chrome's Local Network Access prompt; the service operator allows the origin for the logdata API (§3). **Done 2026-10-27**, except the service: the hub's default list has the hosted app's origin (exactly: not other ports, `http://`, or longer names; tested). Chrome 156 (`web/e2e/hosted.spec.ts`, the production build served as `https://uclog.unitcircle.ca`): a public page's `ws://127.0.0.1` is blocked until the site has the "local-network-access" permission (Chrome asks; once denied the socket closes with 1006 before opening), so the app now adds a hint to that error (allow "Local network access" in the site's settings); with the permission, a local hub admits the hosted origin with no approval and turns away other public origins. Left for the service operator: allow `https://uclog.unitcircle.ca` for token requests to `fw.unitcircle.ca/logdata` | S |
| 11 | Screenshot generation (Playwright, egui snapshots, ratatui SVG) wired into the docs build. **Done 2026-10-27**: `website/build.py screenshots` runs the web app's (`web/e2e/screens.spec.ts`), VS Code's (`web/apps/vscode/e2e/screens.spec.ts`, from the VSIX), the GUI's (egui_kittest with wgpu, ignored test `gui_screens`) and the TUI's (ratatui `TestBackend` → SVG, ignored test `tui_screens`) against the simulated device, in about 40 s, and copies the chosen ones to `website/docs/assets/screens/` (committed, light and dark where the host has both; shown on "The hosts"). Not run in CI: the GUI needs a GPU and VS Code a display, so they are regenerated by hand when a host's look changes | M |
| 12 | Docs: getting started, using the hosts, logdata and services, security. **Done 2026-10-27**: *Getting started* (install each host, with the release archives' names and the Linux groups and udev rules, Windows not code-signed yet (SmartScreen), the macOS Bluetooth permission; the simulated device; a board); *Hosts*: an overview with the device addresses, and pages for VS Code, `uclog` (GUI, TUI, commands, settings and where they live, environment, platform notes), the hub (behaviour, a lab or CI hub) and Python; *Logdata* (the app hash, the lookup order, the cache and clearing it, the web app's own cache, offline, the logdata service: the request, the 5-minute link, domain and token per host, refused tokens, CORS and `null` origins); *Security* (hub admission and the Origin check, binding, mDNS, TLS needs a public-CA certificate, Local Network Access, the semihost box per host: no SYSTEM/unlink/rename, one folder; tokens per host; firmware updates). Written from the code and the READMEs, checked against `--help` and the sources. Not claimed: a `uclog logdata upload` command (plan README §5.3; not implemented: uploading is the service's API) | L |
| 13 | Docs: from device code to screen; adding ucLog to a project (Makefile, CMake, Zephyr) with the minimal examples (`device/examples/`), compiled in CI. **Done 2026-10-27**: `device/examples/` (README): `baremetal/` on the NUCLEO-L452RE-P, the same `main.c` and `port.c` (the board and the port in CMSIS registers: USART2 at 1 Mbaud, circular RX DMA, TX DMA from the library's ring, SysTick; about 250 lines) built by `make` (`uclog.mk`) and by CMake (`add_subdirectory`, `uclog_add_logdata`, toolchain file `cortex-m4f.cmake`), 10 KB of flash; `zephyr/` (replaces `device/app`, which had commented-out code and an `#if 0` block): `LOG_INF` through the custom header, the LED from the devicetree, the L452 overlay with circular RX DMA, the nRF52840-DK overlay; `uclog-target0` unless another target snippet is given. Checked on the L452 with the Python library and the hub (device info, backlog, enums, floats, the terminal commands, logdata from the cache without the ELF), make and CMake builds both, `LOG_FATAL`'s polled flush and reset (from a scratch copy), and the Zephyr example; built with Ubuntu's Arm GCC as CI does, and the Zephyr matrix (L452, nRF ucuart/ucusb/ucuart_z, options with `uclog-target1;uclog-shell`) on Zephyr 4.3 (3.7.2 and 4.4.2 in CI). Docs: *Device → From device code to screen* (how a log travels; levels; every argument kind, enums, symbols and memory, with the output captured from the test firmware's selftest; terminal, plots, semihost, commands, FW update) and *Adding ucLog to a device* (start-up, the port interface, the linker additions and target digit, Makefile/CMake/by hand, Zephyr module/devicetree/snippets/Kconfig, links, build outputs, both examples' source included). Fixed on the way: the target0 and target1 snippets disagreed (`LOG_MODE_MINIMAL=n` in one; inline comments, the no-op `UC_LOG_SERVER`); the bare-metal build links the library's syscalls, so newlib's stub warnings are gone. Known: Zephyr apps' logs show `CMAKE_SOURCE_DIR/…` file names (Zephyr's own prefix map) | L |
| 14 | Docs: embedding, with `web/apps/demo` as the reference (§5); the generated `<uc-log>` API reference. **Done 2026-10-27**: *Embedding → In a web page* (the script URLs, what the page needs: secure context, iframe `allow`, CSP, hubs and Local Network Access; logdata and tokens; scripting; styling) with both examples' source included from the repository (`pymdownx.snippets`, a missing file fails the build): `web/examples/embed` (the portal) and `web/apps/demo`, which now imports only `@uclog/element` (its `dist/` in builds; its source under `vite` dev). *Element reference* is generated: JSDoc on the class (`@fires`, `@csspart`, `@cssprop`, `@internal`) → `cem analyze` (Custom Elements Manifest analyzer 0.11.0, a plugin drops Lit's state fields) → `website/build.py`; the element build also ships `dist/custom-elements.json` (`customElements` in package.json). Found on the way: the views' parts (`table`, `terminal`, `toolbar`, `plot`, …) were documented but not reachable from the page (now `exportparts`, tested), and the portal example filtered logs on a field that does not exist (`r.lvl`). `e2e/embed.spec.ts` now also runs `examples/embed` from its own server (3 ways of loading the bundle); Playwright builds the element first (`e2e/global-setup.ts`) | M |
| 15 | Python: `python/examples/` (connect and filter logs, send commands, a pytest hardware fixture, decode a capture), run against the loopback device in CI; API reference from docstrings. **Done 2026-10-27**: `capture_logs.py` filters by level, source and text; new `decode_capture.py` (a raw capture through `FrameScanner`, `cobs_decode`, `mux_decode` and `LogData.decode`, no device or hub) and `pytest/` (a session `dev` fixture with `--uclog-device`/`--uclog-host`/`--uclog-logdata`, a `no_errors` fixture, four generic tests). `tests/test_examples.py` runs all five against `uclog-hub` with `loopback:` (so in `python.yml` on three OSes); the capture is the raw bytes of a raw-TCP hub client. The simulated device now answers CBOR requests on port 0 (`get-info`, `echo`, else `error`), so `command.py` and the hosts' CBOR buttons have something to talk to (Rust test; the web app's simulated device does not yet). *Python reference* is generated by `website/build.py` from `uclog.__all__` (grouped; a name missing from the groups fails the build): signatures, docstrings as Markdown (lists, indented examples as fenced code), dataclass fields, properties and methods, module helpers and constants; docstrings added where missing (`Resolver` with its lookup order, `connect()`'s remaining parameters, and seven more). The docs' *Python* page lists the examples and includes the pytest and capture ones | M |
| 16 | `bench` firmware command, the results script and the performance pages (§6), on the three boards | M |
| 17 | Download page: picks the right release asset for the visitor's OS/arch; checksums; signing status | S |
| 18 | Release notes and the compatibility table generated from tags | S |
| 19 | Go public: push this repository to `UnitCircleInc/uclog` with the device history merged in (task 3), keeping its `v1`–`v3` tags; release `v4` and `device-v4`; make it public; announce from www.unitcircle.ca | S |
| 20 | Future: `Origin: null` logdata requests to fw.unitcircle.ca from the single-file app (§3.1). Explore with the service: for example, accept `null` on token requests when the token is one meant for it (a scoped, read-only logdata token, or a token flag the customer sets); what a leaked token then exposes; whether rate limits differ | S |

Order: task 2 now. Then 1 and 3, then 4–7 (one device library), 8 and 10 alongside. Then 9, 9a, 9b and 11, then the content
(12–16), then 17–19 (20 is not needed for going public). Plan 07's publishing jobs (PyPI, Marketplace, Open VSX, npm) must be done
before task 19.

## 8. Open questions

None at the moment. Decided 2026-10-25: option A (§2.2), SoloVer starting at 4 (§2.2a), the
offline web app with a single-file download (§3.1), versioned docs (§3.2).
