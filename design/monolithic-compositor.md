# Monolithic Gnoblin executable

## Goal

Install one Gnoblin compositor executable. `gnoblin-mutter` must not be a
separate user-facing command or an independently installed compositor binary.
The desktop entry should start `gnoblin`, and that executable should own the
Mutter compositor and Gnoblin's session runtime.

Keep utilities such as `gnoblinctl` and session helpers separate only where
they need to run as independent client or service processes. Recovery and
developer interfaces should be in the compositor process, consistent with the
existing recovery UI direction.

## Current implementation

The source build now produces one compositor executable named `gnoblin`. Its
entry point selects the session guardian or Mutter compositor role, and both
roles use the existing inherited private runtime channel. The session
installer leaves `gnoblinctl` and the idle helper as separate client/service
programs.

## Migration direction

Recovery notices and the developer console are rendered by one Dear ImGui
overlay in the Gnoblin compositor plugin, linked directly into the executable.
The plugin is selected by its built-in GType; startup does not load
`libgnoblin.so`. No recovery or console executable
is built or installed. The console exposes Lua only. Its evaluator and
configuration reload action use native control, so evaluation retains the
runtime's normal validation and operation queue.

The console restores at most 200 submitted commands from
`$XDG_STATE_HOME/gnoblin/console-history.ini` (or
`~/.local/state/gnoblin/console-history.ini`). Saving is coalesced through an
asynchronous GIO replacement write, so submitting a command does not perform
file I/O on the compositor input path. History is diagnostic state and is not
part of the Lua configuration.

Lua completion and result inspection use bounded native-control requests.
Inspection nodes carry pre-rendered labels, values, children, and truncation
markers, so expanding a result does not execute Lua in the compositor UI
thread. The UI independently limits decoded depth, node count, child count,
and text size before drawing it.

## Acceptance

- A fresh session install contains one compositor executable named `gnoblin`.
- The login entry starts that executable without a `gnoblin-mutter` path.
- Session supervision, Lua configuration recovery, native control, and
  compositor startup continue to work through the single executable.
- `gnoblinctl` continues to be discoverable from a registered source install.
- Recovery and the Lua console run inside the compositor without separate UI
  executables.
- The optional portal backend remains an independent extension.
