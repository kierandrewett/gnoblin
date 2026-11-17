# Start Gnoblin and choose a shell

`gnoblin` starts the session: the compositor and the shell. Which shell runs is decided once, before anything starts, by
looking in a fixed list of places in order. A session never starts without a shell: if there is none, or the choice is not
clear, `gnoblin` says why and exits before a compositor opens a blank screen.

```sh
gnoblin                          # the shell Gnoblin finds
gnoblin ./libmyshell.so          # this file
gnoblin --shell tiler            # the installed shell called tiler
gnoblin --list-shells            # every shell Gnoblin can see, and the one that would run
```

## The command line

```text
gnoblin [--shell NAME] [--watch] [--isolate] [--list-shells] [PATH]
```

- `PATH` is a file or a directory, always a path: relative to the directory you run `gnoblin` in, so `./myshell.so` and
  `shell.py` both work. It is a native library (`libmyshell.so`), a Python, JavaScript or TypeScript file, a directory that
  holds a `shell.<ext>`, or a launcher file (`tiler.toml`, see [installed shells](#installed-shells)).
- `--shell NAME` is always a name: an installed shell. A path and a name together are a usage error (exit 2).
- `--watch` reloads the shell when it changes. With no `PATH` it watches the shell Gnoblin finds. See
  [Reload and watch](#reload-and-watch).
- `--isolate` runs the shell in a process of its own. See [Isolate the shell](#isolate-the-shell).
- `--list-shells` prints the table below and exits 0.

A word with no `/` that is not a file is an error that asks whether you meant `--shell`:

```text
gnoblin: tiler: no such file (from the command line).
Did you mean --shell tiler?    (a path needs a `/`, as in ./tiler)
```

`gnoblin --role ROLE` and `--language LANGUAGE` are what the session starts its own children with. You do not need them.

## Where the shell comes from

The first place in this list that gives exactly one shell wins.

| Tier | Source                                                                                             | What it names                                                                                            |
| ---- | -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| 1    | The command line: `PATH`, or `--shell NAME`                                                        | The path as given, or the installed shell of that name.                                                  |
| 2    | `GNOBLIN_SHELL`                                                                                    | A path when the value has a `/`, else a name. Empty is ignored.                                          |
| 3    | `[shell]` in `$XDG_CONFIG_HOME/gnoblin/config.toml`                                                | `default = "name"`, or `path = "..."`. Both together is an error.                                        |
| 4    | `shell.so`, `shell.py`, `shell.js`, `shell.mjs`, `shell.ts` or `shell.mts` in the config directory | The file that is there.                                                                                  |
| 5    | `$XDG_DATA_HOME/gnoblin/shells/` (`~/.local/share/gnoblin/shells/`)                                | An [installed shell](#installed-shells).                                                                 |
| 6    | `gnoblin/shells/` in each `$XDG_DATA_DIRS` entry, in order (`/usr/local/share:/usr/share`)         | An installed shell. The first directory that has one decides, so an earlier directory hides a later one. |
| 7    | The packaged default: `<prefix>/share/gnoblin/default-shell/shell.<ext>`                           | What a fresh install runs. Found from where `gnoblin` is installed, so a relocated prefix finds its own. |

The config directory is `$XDG_CONFIG_HOME/gnoblin` when that is an absolute path, else `~/.config/gnoblin`. A relative
`XDG_CONFIG_HOME`, `XDG_DATA_HOME` or `XDG_DATA_DIRS` entry is ignored, as the XDG specification says.

Three rules keep the choice honest:

- **A request that cannot be met is an error, never a fall-through.** If tier 1, 2 or 3 names a shell that is not there,
  Gnoblin stops. A person who named a shell and got a different one has been misled.
- **A tier with nothing in it is skipped.**
- **Equals are not ranked.** Two shells in one tier are an error that lists them, except that inside a `shells/` directory
  (tiers 5 to 7) a shell named `default` wins over the others.

A name (`--shell tiler`, `GNOBLIN_SHELL=tiler`, `default = "tiler"`) is looked up in tiers 5, 6 and 7, in that order. The
packaged default is the shell named `default`.

A `config.toml` has one table:

```toml
[shell]
default = "tiler"        # the name of an installed shell, or
# path = "~/dev/tiler/shell.ts"   # absolute, ~/..., or relative to this file
# language = "ts"        # only to override how the file is detected
# isolate = true         # run the shell in its own process, whichever tier chose it
```

An unknown key is a warning. `isolate` is read whichever tier chose the shell; the other keys are read when the lookup
reaches tier 3. A `config.toml` that cannot be read ends the start only when the lookup reaches it; when a higher tier
chose the shell it is a warning.

### Installed shells

In a `shells/` directory a shell is one of:

- a module file: `tiler.so`, `tiler.py`, `tiler.js`, `tiler.mjs`, `tiler.ts` or `tiler.mts`;
- a directory `tiler/` that holds a `shell.<ext>`, for a shell of several files;
- a link to either;
- a launcher file, `tiler.toml`, a few lines that point at a shell without copying it:

```toml
# /usr/share/gnoblin/shells/tiler.toml
module = "libtiler.so"           # relative to <libdir>/gnoblin/shells/, which is correct on every architecture
# path = "/opt/tiler/shell.py"   # or: absolute, or relative to this file
# language = "native"            # only to override how the file is detected
description = "A tiling shell"
```

The name is the file name without its extension, the directory's name, or the link's own name. The same name in an
earlier tier hides a later one; `--list-shells` marks the hidden one `shadowed`.

## See what Gnoblin sees

```sh
gnoblin --list-shells
```

```text
NAME     TIER  LANGUAGE  DEFAULT  LOCATION
tiler    5     native    yes      /home/me/.local/share/gnoblin/shells/tiler.toml -> /opt/tiler/libtiler.so
lines    6     python             /usr/share/gnoblin/shells/lines.py
default  7     ts                 /usr/share/gnoblin/default-shell/shell.ts  (packaged)
```

One row per shell in every place, in the order Gnoblin looks, and `yes` marks the one that would run now. A launcher file
shows its target after `->`. `gnoblinctl shell list` prints the same table and needs no session.

## When there is no shell, or too many

With nothing installed, and no packaged default, Gnoblin says where it looked and what to do, exits 1 and starts nothing:

```text
gnoblin: no shell found.

Looked in this order:
  1. command line            not given
  2. GNOBLIN_SHELL           not set
  3. /home/me/.config/gnoblin/config.toml   missing
  4. /home/me/.config/gnoblin/shell.{so,py,js,mjs,ts,mts}   none present
  5. /home/me/.local/share/gnoblin/shells/  missing
  6. /usr/local/share/gnoblin/shells/  missing;  /usr/share/gnoblin/shells/  empty
  7. /usr/share/gnoblin/default-shell/shell.*   not installed

Run a shell directly:     gnoblin ./myshell.so
Pick an installed shell:  gnoblin --shell NAME    (gnoblin --list-shells)
Set a default:            [shell] default = "NAME"  in /home/me/.config/gnoblin/config.toml
```

Several equal candidates in one tier are listed with the same block (the tier that has them says how many) and the same three
remedies:

```text
gnoblin: several shells are equally preferred:
  lines  /home/me/.local/share/gnoblin/shells/lines.py  (python)
  tiler  /home/me/.local/share/gnoblin/shells/tiler.toml -> /opt/tiler/libtiler.so  (native)
```

A name that is not installed says so, offers the nearest installed name when one is within two edits, lists what is
installed and where Gnoblin looked.

A shell that is found and cannot load (its `init` fails, it was built for another ABI, a library is missing, a script throws)
prints the loader's own reason: the `dlopen` message for a missing library, both versions for an ABI mismatch, and
`file:line:column` for a script.

The session runs on with the compositor's own window management, and the shell is reloadable once the file is fixed.

## Reload and watch

A shell is replaced while it runs. Windows stay open on the same Wayland connection and surface, a shell's nodes, kept state
and running programs carry over (see `design/modular-framework.md`, section 8), and a replacement that cannot take over leaves
the running shell serving.

- **A native shell** is a library you rebuild. `gnoblin --watch ./target/debug/libmyshell.so` reloads it each time the file is
  rewritten, after 100 ms of quiet. The compositor never runs `cargo`: build with `cargo build`, or keep
  `cargo xtask watch-shell PROJECT` running, which builds on every source change and keeps a failed build's compiler output for
  `gnoblinctl shell status`.
- **A script** reloads when any file it imported changes: the shell's runtime watches the import set, with or without
  `--watch`. `GNOBLIN_JS_WATCH=0` turns that off for JavaScript and TypeScript.
- **On request**, `gnoblinctl shell reload` reloads the shell and waits for the outcome: exit 0 and `reloaded: generation 3`,
  or exit 1 and the reason (`reload of /x/libshell.so failed, still running generation 2: ...`).
- `gnoblinctl shell status` says which generation runs and how the reloads went.
- `gnoblinctl shell watch [PATH]` is the same loop from the command line: it reloads the running shell each time `PATH` (default:
  the shell Gnoblin finds) is rebuilt, until you interrupt it. `--once` exits after the first reload with its code.

A failed reload is logged once with its reason and changes nothing: the old generation, its windows, its keys and its timers
carry on, and the next change tries again.

## Isolate the shell

By default the shell runs in the compositor's process, on a thread of its own: the fastest and simplest arrangement. A shell
that misbehaves cannot stall the compositor: a callback that never returns costs the shell its own events, and the lock screen
falls back to the compositor's built-in card.

A **crash** in native shell code, though, takes the process with it.

`gnoblin --isolate` (or `[shell] isolate = true`) runs the shell as a child of the session instead, over the compositor's
socket. The guardian restarts it with a back-off if it dies, and a segfault in shell code is the shell's alone: the compositor,
your windows and your clients carry on. `gnoblinctl shell reload` reaches only a shell in the compositor's process; an
isolated shell reloads with `--watch`.

## The packaged default

A package installs a starter shell as `<prefix>/share/gnoblin/default-shell/shell.ts`: the stock desktop as a shell you can
read. It is what runs on a fresh install, and it is only the last tier: write a `shell.ts` with `gnoblinctl config init` and
that one runs instead. See the [TypeScript shell](/typescript-shell).
