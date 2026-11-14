# Configuration files

Your configuration is one TypeScript file, `init.ts`, in `$XDG_CONFIG_HOME/gnoblin`, or in `~/.config/gnoblin` when
`XDG_CONFIG_HOME` is unset. `GNOBLIN_CONFIG` names a different file instead. The shell takes the first of `init.ts` and
`init.js` that exists in that directory.

Write a commented starter, with the editor files that give you completion, once:

```sh
gnoblinctl config init
```

`config init` never replaces a config. Add `--force` to write the starter over an existing `init.ts`.

Check a file without a running session, and print what the compositor last accepted:

```sh
gnoblinctl check init.ts
gnoblinctl config get
```

A save reloads a running session, and `gnoblinctl config reload` does it on request and prints the error with its file,
line and column when the config is refused. A failed reload keeps the running config. The rest of the story is in the
[TypeScript shell](/typescript-shell#where-the-config-lives), and a config split over files is in
[files and load order](/guides/files_and_load_order).
