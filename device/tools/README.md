# Build-time tools

Called by the Zephyr CMake helpers in `../cmake/` and by the bare-metal Makefiles in
`../../firmware/`:

| Script | Used by | Does |
|---|---|---|
| `logdata.py` | `log.cmake`, the Makefiles | the `.logdata` CBOR from the ELF (Python standard library and `objdump -Wi`; `-o` or `--ofile`) |
| `hash.py` | `apphash.cmake`, the Makefiles | the app hash of the image (`-f`: the whole unsigned image; else after the 512-byte signature header); prints it in hex |
| `cachelogdata.py` | `log.cmake` | copies the `.logdata` into the hosts' cache (`$UCLOG_CACHE_DIR`, else `$XDG_CACHE_HOME/uclog`, else `~/.cache/uclog`), named by app hash |
| `sbl.py` (with `ihex.py`, `sss.py`, `gf2.py`, `zbase32.py`) | `signature.cmake` | signs the image |

The legacy host scripts (`uclog.py`, `uccon.py`, `cobs.py`) were retired when this library moved
into the ucLog repository: use the `uclog` command (`uclog term`, `uclog tui`, `uclog gui`), the
`uclog` Python package, the web app or the VS Code extension. They remain in the `v1`–`v3` tags.
