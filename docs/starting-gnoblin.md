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

| Tier | Source                                                                                             | What it names                                                            |
| ---- | -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| 1    | The command line: `PATH`, or `--shell NAME`                                                        | The path as given, or the installed shell of that name.                  |
| 2    | `GNOBLIN_SHELL`                                                                                    | A path when the value has a `/`, else a name. Empty is ignored.          |
| 3    | `[shell]` in `$XDG_CONFIG_HOME/gnoblin/config.toml`                                                | `default = "name"`, or `path = "..."`. Both together is an error.        |
| 4    | `shell.so`, `shell.py`, `shell.js`, `shell.mjs`, `shell.ts` or `shell.mts` in the config directory | The file that is there.                                                  |
| 5    | `$XDG_DATA_HOME/gnoblin/shells/`                                                                   | An [installed shell](#installed-shells).                                 |
| 6    | `gnoblin/shells/` in each `$XDG_DATA_DIRS` entry                                                   | An installed shell. The first directory that has one decides.            |
| 7    | `<prefix>/share/gnoblin/default-shell/`                                                            | The shell a fresh install runs, found from where `gnoblin` is installed. |

The config directory is `$XDG_CONFIG_HOME/gnoblin` when that is an absolute path, else `~/.config/gnoblin`.

Without the XDG variables, tier 5 is `~/.local/share/gnoblin/shells/`, and tier 6 is `/usr/local/share/gnoblin/shells/`
then `/usr/share/gnoblin/shells/`. A relative entry in any of the variables is ignored, as the XDG specification says.

Three rules keep the choice honest:

- **A request that cannot be met is an error, never a fall-through.** If tier 1, 2 or 3 names a shell that is not there,
  Gnoblin stops. A person who named a shell and got a different one has been misled.
- **A tier with nothing in it is skipped.**
- **Equals are not ranked.** Two shells in one tier are an error that lists them. Inside a `shells/` directory (tiers 5
  to 7) a shell named `default` wins over the others.

A name (`--shell tiler`, `GNOBLIN_SHELL=tiler`, `default = "tiler"`) is looked up in tiers 5, 6 and 7, in that order. The
packaged default is the shell named `default`.

## One example for each tier

Each example below uses a `tiler` shell. Try one, then run `gnoblin --list-shells` to see which row says `yes` in the
`DEFAULT` column.

**Tier 1: the command line.** Name a file or an installed shell:

```sh
gnoblin ./target/debug/libtiler.so
gnoblin --shell tiler
```

**Tier 2: `GNOBLIN_SHELL`.** Useful in a script or a login-manager entry. A value with a `/` is a path:

```sh
GNOBLIN_SHELL=$HOME/dev/tiler/shell.py gnoblin
GNOBLIN_SHELL=tiler gnoblin
```

**Tier 3: `config.toml`.** Set a default once. `path` is absolute, starts with `~/`, or is relative to the directory that
holds `config.toml`:

```toml
# ~/.config/gnoblin/config.toml
[shell]
path = "~/dev/tiler/shell.ts"
```

`path` can name a shell in any language: `path = "~/dev/tiler/shell.ts"` (TypeScript), `path = "~/dev/tiler/shell.py"` (Python) or `path = "~/dev/tiler/libtiler.so"` (a Rust shell). Use `default = "tiler"` instead to name an installed shell. Setting both is an error:

```text
gnoblin: /home/me/.config/gnoblin/config.toml: `[shell]` sets both `default` and `path`; keep one
```

**Tier 4: a file in the config directory.** Copy or link your shell to the name the session looks for:

```sh
cp target/debug/libtiler.so ~/.config/gnoblin/shell.so      # a native shell
cp tiler.py ~/.config/gnoblin/shell.py                       # or a script
```

**Tier 5: installed for your user.** Put a module file, a directory or a launcher file in your data directory:

```sh
mkdir -p ~/.local/share/gnoblin/shells
cp tiler.ts ~/.local/share/gnoblin/shells/tiler.ts    # TypeScript
cp tiler.py ~/.local/share/gnoblin/shells/tiler.py    # Python
cp libtiler.so ~/.local/share/gnoblin/shells/tiler.so # Rust
gnoblin --shell tiler
```

**Tier 6: installed for the system.** The same, in `/usr/share/gnoblin/shells/` or any other `$XDG_DATA_DIRS` entry. A
package ships its shell this way.

**Tier 7: the packaged default.** Install Gnoblin and choose nothing. With no `shell.*` file of your own, `gnoblin` runs
the stock shell, `<prefix>/share/gnoblin/default-shell/shell.ts`. It is the only shell that ships; it is a TypeScript file,
and `gnoblinctl config init --language python` or `--language rust` writes the same desktop in those languages.

An earlier tier hides a later one, and a request that names something missing is an error. To check which shell a setup
selects without starting a session, run `gnoblin --list-shells`.

## The config file

A `config.toml` has one table:

```toml
[shell]
default = "tiler"        # the name of an installed shell, or
# path = "~/dev/tiler/shell.ts"   # absolute, ~/..., or relative to this file
# language = "ts"        # only to override how the file is detected
# isolate = true         # run the shell in its own process, whichever tier chose it
```

| Key        | Accepted values                                           | Default                | Effect                             |
| ---------- | --------------------------------------------------------- | ---------------------- | ---------------------------------- |
| `default`  | An installed shell's name                                 | none                   | Names the shell to run. Tier 3.    |
| `path`     | An absolute path, `~/...`, or a path relative to the file | none                   | The shell to run. Tier 3.          |
| `language` | `native`, `python`, `js`, `ts`, or a runtime's name       | detected from the file | How to load the file.              |
| `isolate`  | `true` or `false`                                         | `false`                | Runs the shell in its own process. |

An unknown key is a warning. `isolate` is read whichever tier chose the shell; the other keys are read when the lookup
reaches tier 3. A `config.toml` that cannot be read ends the start only when the lookup reaches it. When a higher tier chose
the shell it is a warning.

### Installed shells

In a `shells/` directory a shell is one of:

- a module file: `tiler.so`, `tiler.py`, `tiler.js`, `tiler.mjs`, `tiler.ts` or `tiler.mts`;
- a directory `tiler/` that holds a `shell.<ext>`, for a shell of several files;
- a link to either;
- a launcher file, `tiler.toml`: a few lines that point at a shell without copying it.

A launcher file has these keys. Set exactly one of `module` and `path`:

| Key           | Accepted values                                               | Effect                                                                 |
| ------------- | ------------------------------------------------------------- | ---------------------------------------------------------------------- |
| `module`      | A path relative to `<libdir>/gnoblin/shells/`                 | A native shell library. Correct on every architecture.                 |
| `path`        | An absolute path, `~/...`, or a path relative to the launcher | A script or a library anywhere on disk.                                |
| `language`    | `native`, `python`, `js`, `ts`, or a runtime's name           | Overrides detection. A file that begins with the ELF header is native. |
| `description` | Any text                                                      | A note for people reading the file.                                    |

```toml
# /usr/share/gnoblin/shells/tiler.toml
module = "libtiler.so"
description = "A tiling shell"
```

The name is the file name without its extension, the directory's name, or the link's own name. The same name in an earlier
tier hides a later one, and `--list-shells` marks the hidden one `shadowed`. A launcher whose target is missing shows as
`unusable` in the list and is refused if you choose it.

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

One row per shell in every place, in the order Gnoblin looks:

- `yes` in the `DEFAULT` column marks the shell that would run now. `shadowed` marks one that an earlier tier hides.
- A launcher file shows its target after `->`. One that cannot be used ends in `[unusable: ...]` with the reason.
- `gnoblinctl shell list` prints the same table and needs no session.
- `--list-shells` cannot be combined with `--watch` or `--isolate` (exit 2).

## When there is no shell, or too many

With nothing installed and no packaged default, Gnoblin says where it looked and what to do, exits 1 and starts nothing:

```text
gnoblin: no shell found.

Looked in this order:
  1. command line            not given
  2. GNOBLIN_SHELL           not set
  3. /home/me/.config/gnoblin/config.toml   missing
  4. /home/me/.config/gnoblin/shell.{so,py,js,mjs,ts,mts}   none present
  5. /home/me/.local/share/gnoblin/shells/  missing
  6. /usr/local/share/gnoblin/shells/  missing;  /usr/share/gnoblin/shells/  missing
  7. /usr/share/gnoblin/default-shell/shell.*   not installed

Run a shell directly:     gnoblin ./myshell.so
Pick an installed shell:  gnoblin --shell NAME    (gnoblin --list-shells)
Set a default:            [shell] default = "NAME"  in /home/me/.config/gnoblin/config.toml
```

Several equal candidates in one tier are listed with the same block. The tier that holds them says how many, and the later
tiers say `not reached`:

```text
gnoblin: several shells are equally preferred:
  lines  /home/me/.local/share/gnoblin/shells/lines.py  (python)
  tiler  /home/me/.local/share/gnoblin/shells/tiler.ts  (ts)

Looked in this order:
  1. command line            not given
  2. GNOBLIN_SHELL           not set
  3. /home/me/.config/gnoblin/config.toml   missing
  4. /home/me/.config/gnoblin/shell.{so,py,js,mjs,ts,mts}   none present
  5. /home/me/.local/share/gnoblin/shells/  2 shells
  6. /usr/local/share/gnoblin/shells/  not reached
  7. /usr/share/gnoblin/default-shell/shell.*   not reached

Run a shell directly:     gnoblin ./myshell.so
Pick an installed shell:  gnoblin --shell NAME    (gnoblin --list-shells)
Set a default:            [shell] default = "NAME"  in /home/me/.config/gnoblin/config.toml
```

Choose one of the three remedies: run one by path, pick one by name, or name a default in `config.toml`. Two shells in the
config directory (a `shell.py` and a `shell.ts`) are an ambiguity of tier 4 in the same form.

A name that is not installed says so, offers the nearest installed name when one is within two edits, lists what is
installed and where Gnoblin looked:

```text
gnoblin: no shell named "tilr" (from the command line).
Did you mean "tiler"?

Installed shells:
  lines  /home/me/.local/share/gnoblin/shells/lines.py
  tiler  /home/me/.local/share/gnoblin/shells/tiler.ts

Looked in:
  /home/me/.local/share/gnoblin/shells/
  /usr/local/share/gnoblin/shells/  (missing)
  /usr/share/gnoblin/shells/  (missing)
```

The first line ends with where the name came from: `(from the command line)`, `(from GNOBLIN_SHELL)` or `(from [shell] in
/home/me/.config/gnoblin/config.toml)`.

A path that does not exist is its own error:

```text
gnoblin: ./nothere.so: no such file (from the command line).
```

With no shell installed at all, a name says so: `No shells are installed.` instead of the list.

A shell that is found and cannot load prints the loader's own reason. That covers a failed `init`, a different ABI, a
missing library and a script that throws:

- A missing library gives the `dlopen` message.
- An ABI mismatch gives both versions.
- A script gives `file:line:column`.

The session runs on with the compositor's own window management, and the failed start is neither quiet nor final:

- The guardian's log carries the loader's text: `[guardian] configuration revision 0 rejected: the shell failed to start: ...`.
- `gnoblinctl shell status` prints `state: failed` and the same text (`"state": "failed"` and `"error"` with `--json`).
- Fixing the file loads the shell in the same session, with no restart. A script is loaded when it is saved, a native
  module with `--watch`, and any shell with `gnoblinctl shell reload`, which exits 1 with the new text while the file
  still fails.
- `GNOBLIN_JS_WATCH=0` turns the save-to-load off for JavaScript and TypeScript.
- An isolated shell (`--isolate`) exits with the failure instead, and the guardian restarts it under its back-off.

## Reload and watch

A shell is replaced while it runs. Windows stay open on the same Wayland connection and surface, a shell's nodes, kept state
and running programs carry over, and a replacement that cannot take over leaves
the running shell serving. [Write a shell in Rust](rust-shell.md#reload-while-it-runs) lists what a native shell keeps.

- **A native shell** is a library you rebuild. `gnoblin --watch ./target/debug/libmyshell.so` reloads it each time the file is
  rewritten, after 100 ms of quiet. The compositor never runs `cargo`: build with `cargo build`, or keep
  `cargo xtask watch-shell PROJECT` running, which builds on every source change and keeps a failed build's compiler output for
  `gnoblinctl shell status`.
- **A script** reloads when any file it imported changes: the shell's runtime watches the import set, with or without
  `--watch`. `GNOBLIN_JS_WATCH=0` turns that off for JavaScript and TypeScript. A script that failed to start is watched
  the same way, by its entry file.
- **On request**, `gnoblinctl shell reload` reloads the shell and waits for the outcome: exit 0 and `reloaded: generation 3`,
  or exit 1 and the reason (`reload of /x/libshell.so failed, still running generation 2: ...`). An isolated shell has no
  in-process shell to reload, and the command says so: it reloads with `--watch`.
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
read. It is what runs on a fresh install, and it is only the last tier. Any shell file of your own runs instead:

- `gnoblinctl config init` writes a starter `shell.ts` (TypeScript).
- `gnoblinctl config init --language python` writes a starter `shell.py` (see the [Python shell](/python-shell)).
- `gnoblinctl config init --language rust` writes a Cargo project for a Rust shell crate (see the [Rust shell](/rust-shell)).
