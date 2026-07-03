# Config Parser

`src/config/` owns the C implementation of the `gnoblin.conf` reader. It is
compiled into the Mutter protocol overlays (its `manifest` copies it to
`src/wayland/` in the Mutter tree), where it gates protocols and other
overlay behaviour.

## Files

- `gnoblin-config.c` is the parser and in-memory section table.
- `gnoblin-config.h` is the public interface used by the overlay code.
- `manifest` copies the parser into the patched Mutter tree during overlay
  application.

## Interface

The public interface is intentionally small:

- `gnoblin_config_path()` resolves `$GNOBLIN_CONFIG`, then
  `$XDG_CONFIG_HOME/gnoblin/gnoblin.conf`.
- `gnoblin_config_reload()` reparses the file and swaps the loaded table.
- `gnoblin_config_get_bool/int/string()` return the last value for a key.
- `gnoblin_config_get_list()` returns all repeated values for one key.
- `gnoblin_config_get_keys()` returns keys in file order, including repeats.

Compiled into gnoblin's Mutter protocol overlays (`src/protocols/`), which read
`gnoblin.conf` to gate each Wayland protocol on/off (`[protocols]` keys — see
`src/data/gnoblin.conf.example`). Unset keys fall back to the caller's default.

## Grammar Contract

- Lines trim leading space/tab and trailing space/tab/CR/LF.
- Empty lines and lines starting with `#` or `;` after trimming are comments.
- Section lines start with `[` and use the first later `]`; missing `]` drops
  that line.
- Key/value lines use the first `=`. Empty keys are ignored. Repeated keys are
  retained in file order; scalar lookups return the last value.
- A value whose first non-space byte is a single or double quote uses bytes up
  to the next same quote, drops the rest of the line, and does not process
  escapes.
- Unquoted values strip a `#` inline comment only when the `#` is introduced by
  space/tab and is outside a simple quoted span. `;` is data inline.

## Verification

```sh
gcc -fsyntax-only src/config/gnoblin-config.c -I src/config $(pkg-config --cflags glib-2.0)
cc tests/config-test.c src/config/gnoblin-config.c -I src/config $(pkg-config --cflags --libs glib-2.0) -o /tmp/gnoblin-config-test
/tmp/gnoblin-config-test
```

Or just run `just test-config`.
