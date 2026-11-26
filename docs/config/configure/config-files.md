# Configuration files

Your configuration is one shell file, in `$XDG_CONFIG_HOME/gnoblin`, or in `~/.config/gnoblin` when `XDG_CONFIG_HOME`
is unset. Its name depends on the shell language, and each name is found the same way:

- `shell.ts` for a TypeScript shell.
- `shell.py` for a Python shell.
- `shell.so` for a Rust shell library built from a project.

Other places can name the shell instead:

- `GNOBLIN_SHELL` names a different file, or an installed shell.
- `[shell]` in `config.toml` picks one by name or path.
- Two shell files in one directory are an error, not a choice.

The order is in [Where the config lives](/typescript-shell#where-the-config-lives), and it applies to all three shells.

Write a commented TypeScript starter, with the editor files that give you completion, once:

```sh
gnoblinctl config init
```

`config init` never replaces a config. Add `--force` to write the starter over an existing `shell.ts`. For a Python or
Rust shell, start from the [Python shell](/python-shell) or the [Rust shell](/rust-shell) page instead.

Check a `.ts`, `.py` or `.js` file without a running session, and print what the compositor last accepted:

```sh
gnoblinctl check shell.ts
gnoblinctl config get
```

A save reloads a running session, and `gnoblinctl config reload` does it on request and prints the error with its file,
line and column when the config is refused. A failed reload keeps the running config. The rest of the story is in
[Where the config lives](/typescript-shell#where-the-config-lives), and a config split over files is in
[files and load order](/guides/files_and_load_order).
