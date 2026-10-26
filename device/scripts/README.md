# Build-time tools

Called by the CMake helpers in `lib/`:

| Script | Used by | Does |
|---|---|---|
| `logdata.py` | `log.cmake` | the `.logdata` file from the ELF |
| `cachelogdata.py` | `log.cmake` | copies the `.logdata` into the host cache, keyed by app hash |
| `hash.py` | `apphash.cmake` | the app hash of the image |
| `sbl.py` (with `ihex.py`, `sss.py`, `gf2.py`, `zbase32.py`) | `signature.cmake` | signs the image |

The legacy host scripts (`uclog.py`, `uccon.py`, `cobs.py`) were retired when this library moved
into the ucLog repository: use the `uclog` command (`uclog term`, `uclog tui`, `uclog gui`), the
`uclog` Python package, the web app or the VS Code extension. They remain in the `v1`–`v3` tags.
