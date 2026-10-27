# Theme

The look is Unit Circle's "Sumi & Ai" brand, copied from
[UnitCircleInc/uc-www](https://github.com/UnitCircleInc/uc-www) at commit `fe4e402`:

| Here | From uc-www |
|---|---|
| `zensical.toml`: `[project.theme]` and its palettes | the same section |
| `overrides/` (`main.html`, `.icons/brand/enso.svg`) | `overrides/` |
| `docs/stylesheets/extra.css` | `docs/stylesheets/extra.css` |
| `docs/assets/favicon.svg`, `favicon.ico`, `apple-touch-icon.png` | `docs/assets/` |

To update: copy those files from a newer uc-www commit, change the commit above, and check the
site with `uv run python build.py serve`.
