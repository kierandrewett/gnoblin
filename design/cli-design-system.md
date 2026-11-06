# CLI design system

One visual language for every Gnoblin command-line tool: `gnoblinctl`, `gnoblin`,
`gnoblin-parity` and `xtask`. The shared code lives in the `gnoblin-cli-style` crate.
The rules for command shape, arguments and exit codes stay in
[cli-development.md](cli-development.md); this document covers how output looks.

## Principles

- **Two audiences, one code path.** Output is for a human on a terminal, or for a
  script. When stdout is not a TTY, when `--json` is given, when `NO_COLOR` is set
  or when `TERM=dumb`, output is plain text with no colour, no cursor movement and
  no spinner. Plain output is stable: scripts may parse it, so changing it is a
  change to a public interface.
- **Colour is never the only signal.** Every state has a word or a glyph beside
  its colour. `ok`, `warn` and `fail` are read correctly in a monochrome terminal
  and in a screenshot taken without colour.
- **Every error says what happened, why it probably happened, and what to do
  next.** The shape is an error block with a `hint:` line (see Components). A
  message that only names the failure is a bug.
- **Exit codes are documented and do not change with styling.** The codes are in
  `docs/cli/index.md` and in cli-development.md. Styling never alters them.
- **Width-aware.** The default width is 80 columns. The minimum is 40. Wrap
  prose, truncate table cells with an ellipsis, and never break a table row across
  lines. Width comes from the terminal when one is attached and from `COLUMNS`
  when it is set; otherwise 80.
- **One visual language.** A heading looks the same in `gnoblinctl status` and in
  `xtask deps`. A new tool reuses the components; it does not invent a style.
- **The machine formats stay byte-stable.** `--json` output, the `[deps]`-style
  line prefixes other tools grep for, and the report files under `design/` are
  not restyled.

## Tokens

### Semantic colours

Colours are semantic names, not fixed values. Each maps to one of the 16 ANSI
colours, so the user's terminal theme decides the actual shade.

| Token     | ANSI         | Used for                                         |
| --------- | ------------ | ------------------------------------------------ |
| `accent`  | bright blue  | section headings, command names, identifiers     |
| `success` | green        | `ok` status, completed operations                |
| `warning` | yellow       | `warn` status, degraded state, deprecated input  |
| `danger`  | red          | `fail` status, error blocks                      |
| `info`    | cyan         | `info` status, hints, event kinds                |
| `muted`   | bright black | secondary text, keys in key-value lists, footers |
| `strong`  | bold         | table headers, emphasised values                 |

No hard-coded RGB and no 256-colour or truecolour escapes. The only SGR codes used
are 0 (reset), 1 (bold), 2 (dim), 30 to 37, and 90 to 97.

### Glyphs

Each glyph has a Unicode form and an ASCII fallback. The fallback is used when the
locale is not UTF-8 or when `TERM=dumb`.

| Name       | Unicode           | ASCII              | Meaning                          |
| ---------- | ----------------- | ------------------ | -------------------------------- |
| `ok`       | `✔`               | `ok`               | success                          |
| `warn`     | `⚠`               | `!`                | degraded, needs attention        |
| `fail`     | `✘`               | `x`                | failure                          |
| `info`     | `ℹ`               | `i`                | information                      |
| `bullet`   | `•`               | `*`                | list item                        |
| `arrow`    | `→`               | `->`               | change from one value to another |
| `ellipsis` | `…`               | `...`              | truncated text                   |
| box        | `─ │ ┌ ┐ └ ┘ ├ ┤` | `- \| + + + + + +` | table rules, section underline   |

Status words are always printed next to the glyph: the glyph alone is never the
signal.

## Components

Examples are plain output (80 columns, no colour). Colour follows the tokens
above.

### Section header

```text
Windows
-------
```

### Key-value list

Keys are muted and aligned. Long values wrap under the value column.

```text
id      12
title   Terminal
state   focused
```

### Table

Columns are aligned. The header is strong. Columns that do not fit shrink, and
the widest column is truncated first.

```text
ID  TITLE         APP ID        FOCUSED
 3  Firefox       firefox       no
12  Terminal      foot          yes
14  Settings      gnome-control no
```

### Status line

One line: status glyph and word, then the subject, then the details.

```text
ok  window 12  focused
warn  output DP-1  scale 2.0 is not an integer
fail  window 14  kill refused
```

### Definition list (help)

Used for `--help` options and arguments: the term is accent, the description
wraps at the width with a hanging indent.

```text
Options:
  --json          Print machine-readable JSON.
  --color <when>  Colour output: auto, always or never.
```

### Error block

Three parts: the summary, an optional detail, and a `hint:` line. The summary
names what happened. The hint says what to do next.

```text
error: the compositor does not know window 12
  the window closed before the request arrived
hint: run `gnoblinctl window list` to see the ids that exist
```

### Inline code and identifiers

Command names, property names and object ids are accent, or inside backticks in
plain output. A value is never quoted by colour alone.

### Waiting

An operation that waits prints a line that says what it waits for.

- On a TTY with a UTF-8 locale: a braille spinner and the text, redrawn in place
  and replaced by the result line when the operation ends.
- Otherwise: a single `waiting for window.focus 12...` line, printed once. Nothing
  is redrawn.

### Event line (watch)

One line per event: a muted timestamp, the event kind in info, then the subject
and the change.

```text
12:04:09  changed  window 12  title "Terminal" -> "vim"
12:04:11  closed   window 14
```

## Adding a command

1. Build the output from `gnoblin-cli-style` components. Do not write ANSI escapes
   or box characters by hand.
2. Take the `Theme` from `Theme::for_stream` with the `--color` choice and pass it
   down; do not read environment variables in the command.
3. Give the plain output a golden test with colour off, ASCII glyphs and width 80,
   and one at width 40.
4. If the command has `--json`, leave that output unchanged and add a regression
   test that it is byte-identical.
5. Write an error block for every failure the command can report. Each block has
   a hint.
6. Document the exit codes in `docs/cli/index.md`.
7. Run `gnoblinctl --help` and the command's `--help`; check both at width 40.
