# Add a shortcut

Add a key that starts a program. Put this in `~/.config/gnoblin/init.ts`:

```ts
gnoblin.input.primary.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

Use an installed terminal. A key is one `bind` call, and your file is the whole configuration, so keep the stock keys you
still want: volume, media and workspace keys are lines of the starter that `gnoblinctl config init` writes.

To change a key that the file already binds, edit that line: a trigger is bound once, and a second `bind` of the same
trigger is refused. This is the stock volume-up key with a smaller step:

```ts
gnoblin.input.primary.bind("XF86AudioRaiseVolume", () => {
    gnoblin.spawn(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "2%+"]);
});
```

| Part                       | Accepted values                                   | Effect                                                                                             |
| -------------------------- | ------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| The trigger                | Modifiers and a key joined by `+`: `Super+Return` | Runs the function when the key is pressed. See [trigger syntax](/guides/shortcuts#trigger-syntax). |
| `gnoblin.spawn([program])` | A program and its arguments, as strings           | Starts the program, passing the arguments, without shell expansion.                                |

Changes apply when you save the file. To drop a stock key, delete its line. Use one string per program argument. See
[key names and conflict handling](/guides/shortcuts) for more.
