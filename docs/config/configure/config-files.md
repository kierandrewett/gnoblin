# Configuration files

Your configuration is one TypeScript file, `shell.ts`, in `$XDG_CONFIG_HOME/gnoblin`, or in `~/.config/gnoblin` when
`XDG_CONFIG_HOME` is unset.

- `GNOBLIN_SHELL` names a different file, or an installed shell, instead.
- `[shell]` in `config.toml` picks one by name or path.
- Two shell files in one directory are an error, not a choice.

The order is in [TypeScript shell](/typescript-shell#where-the-config-lives).

Write a commented starter, with the editor files that give you completion, once:

```sh
gnoblinctl config init
```

`config init` never replaces a config. Add `--force` to write the starter over an existing `shell.ts`.

Check a file without a running session, and print what the compositor last accepted:

```sh
gnoblinctl check shell.ts
gnoblinctl config get
```

A save reloads a running session, and `gnoblinctl config reload` does it on request and prints the error with its file,
line and column when the config is refused. A failed reload keeps the running config. The rest of the story is in the
[TypeScript shell](/typescript-shell#where-the-config-lives), and a config split over files is in
[files and load order](/guides/files_and_load_order).
