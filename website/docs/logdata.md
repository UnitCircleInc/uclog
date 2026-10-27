# Logdata

A device sends logs without their text: each log is the address of its format string and the
arguments. The **logdata** has the rest: every format string, the kind of each argument, and the
image's enums and symbols. The firmware build makes it from the ELF
([Adding ucLog to a device](device/index.md#what-the-build-produces)).

Logdata belongs to one image. The device reports its image's **app hash** (SHA-512 of the image)
when a host connects, and the host uses the logdata with that hash. Rebuilding changes the hash,
so each build's logs are decoded with that build's strings, and a board running an old image is
still decoded correctly.

## Where hosts find it

For each app hash, in this order:

1. **A file you gave it**: `-l` on the command line, *Load logdata…* in the hosts, `logdata=` in
   Python. An ELF (with the logdata embedded as `.logdata_cbor`) or a `.logdata` file.
2. **The cache**: `<app_hash>.logdata` in the cache folder.
3. **A logdata service**, if you set one up (below).
4. In VS Code, also the workspace: a `<app_hash>.logdata` file, or the ELF next to a
   `*.signed.bin` whose hash matches.

Logs whose logdata is not found are not dropped: they are shown as errors with their raw bytes.

## The cache

`~/.cache/uclog/` (`%USERPROFILE%\.cache\uclog\` on Windows), or `$XDG_CACHE_HOME/uclog`, or
`$UCLOG_CACHE_DIR`. It holds `<app_hash>.logdata` files and nothing else, and is shared by
`uclog`, VS Code and the Python library:

- the **device builds** copy each image's logdata there, so a board you just flashed is decoded
  without giving the host anything;
- a file you load is kept there if its name is its app hash, and so is everything downloaded from
  a logdata service;
- the files are small and never change. To clear the cache, delete them (or **ucLog: Clear
  Logdata Cache** in VS Code).

The **web app** cannot read that folder: it keeps its own cache in the browser (IndexedDB, for
uclog.unitcircle.ca), cleared with the site's data. Load the ELF or `.logdata` once and it is kept.

**Offline**, everything above works: only the logdata service needs the network.

## A logdata service

A team's builds can upload their logdata to a service, so that anyone's host finds the logdata of
any image the team built: a board in the field, a colleague's build, a log capture from a
customer. [fw.unitcircle.ca](https://fw.unitcircle.ca/docs) is one; accounts, tokens, uploading
(its API, from a build or CI) and its limits are documented there.

With a **domain** set (your account on the service), a host that has no logdata for an app hash
asks:

```
GET https://fw.unitcircle.ca/logdata/{domain}/{app_hash}
Authorization: Bearer {token}          (only if a token is set)
```

and the service answers with a short-lived download link (5 minutes), which the host follows. The
download is saved in the cache, so each image is fetched once. Only the app hash and the token are
sent; the device's logs never leave your machine.

| Host | Domain and URL | Token |
|---|---|---|
| `uclog` (GUI, TUI, commands) | the settings, or `--logdata-domain`, `--logdata-url` | the settings; kept in the OS keyring (Keychain, Credential Manager, Secret Service) |
| Scripts and CI | `UCLOG_LOGDATA_DOMAIN`, `UCLOG_LOGDATA_URL` | `UCLOG_LOGDATA_TOKEN` |
| VS Code | `uclog.logdata.domain`, `uclog.logdata.url` | **ucLog: Set Logdata Token**; kept in VS Code's secret storage |
| Web app | the settings | the settings; kept for the browser session only, unless you choose to remember it |
| Python | `logdata_domain=`, or the environment | `logdata_token=`, or the environment |
| `<uc-log>` on your page | `logdata-domain`, `logdata-url` | `logdataToken`, set by the page ([Embedding](embedding.md#logdata)) |

A token is only needed if your account requires one to read. If the service refuses a token but
the account allows reading without one, the hosts read without it and offer to remove it.

The web app and `<uc-log>` ask from the browser, so the service must allow the page's origin
(CORS). A page opened from a file (the one-file web app) has no origin the service can allow:
load the logdata there yourself.
