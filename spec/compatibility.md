# Compatibility

Versions are SoloVer (plan 08 §2.2a). The host tools (`uclog`, `uclog-hub`, the Python package,
the web app, `<uc-log>` and the VS Code extension) are tagged `vN` and released together. The
device library is tagged `device-vN` and reports its version as `uclog_version` in the device
info (protocol.md §5.2). The two lines meet in the wire protocol version (protocol.md).

| Device library | Wire protocol | Host tools | Notes |
|---|---|---|---|
| `device-v4` | 1 | `v4` and later | The first release of the monorepo layout |
| `v1`–`v3` (the old device repository) | legacy | not supported | They send the app hash raw on port 63, which protocol 1 uses for semihost requests. Use the legacy host scripts from the same tag |

A new row is added when either line changes the protocol. A host or device release that keeps the
protocol needs no new row: every host `vN` ≥ 4 works with every device `device-vN` ≥ 4 until a row
says otherwise.
