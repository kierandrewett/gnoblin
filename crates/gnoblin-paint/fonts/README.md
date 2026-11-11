# Embedded font

`DroidSans-subset.ttf` is the face the painter falls back to and the only face the golden tests draw with.

- **Font:** Droid Sans, Regular, version 1.00. Digitized data copyright 2007, Google Corporation.
- **Licence:** Apache License, Version 2.0. The full text is in `LICENSE-Apache-2.0.txt`. The font's own `name` table
  carries the same statement (name ids 13 and 14).
- **Source:** the `google-droid-sans-fonts` package's `DroidSans.ttf`.
- **What was changed:** the font was subset and its hinting removed. Nothing else was altered.

The subset keeps U+0020 to U+007E, U+00A0 to U+00FF, the en and em dashes, the single and double curly quotes, the
bullet and the ellipsis, plus `.notdef`. It drops hinting, kerning, substitution and the `gasp` table. To rebuild it:

```sh
pyftsubset DroidSans.ttf \
  --unicodes="U+0020-007E,U+00A0-00FF,U+2013,U+2014,U+2018,U+2019,U+201C,U+201D,U+2022,U+2026" \
  --no-hinting --desubroutinize --layout-features='' --glyph-names --name-IDs='*' --notdef-outline \
  --drop-tables+=GPOS,GSUB,GDEF,gasp --output-file=DroidSans-subset.ttf
```

Changing the subset changes every text golden under `tests/golden/`; regenerate them with
`GNOBLIN_UPDATE_GOLDEN=1 cargo test -p gnoblin-paint` and review the diff.

The font is embedded with `include_bytes!`, so a Gnoblin binary carries Apache-2.0 licensed font data. The repository has
no third-party notices file; this directory is where the notice lives, and distributions that ship a notices file
should list Droid Sans (Apache-2.0) and the licence text beside it.
