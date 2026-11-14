# Complete starter config

Run `gnoblinctl config init` to write the stock default as `~/.config/gnoblin/init.ts`. The file is the whole configuration:
the shell runs it from top to bottom, and with no `init.ts` it runs the same statements. Nothing is layered under your file,
so keep the lines you want to keep.

The file declares four floating workspaces, starts the titlebar, window menu and not-responding alert, and binds the workspace
keys, the strip focus keys, the three-finger swipe, the launcher key and the media keys, each as a call on a live object. To
add a terminal shortcut, add one line after the stock keys; replace `foot` with your terminal.

```ts
gnoblin.input.primary.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

Save the file and the running shell reloads it. A key is bound once at one priority, so a second `bind` of a trigger the
file already binds is refused: delete the stock line first.

To dim unfocused windows, see [dim unfocused windows](/recipes/dim-unfocused-windows). The file does not install or start a
desktop shell; choose one in [shell setup](/bring-your-own-shell).
