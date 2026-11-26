# Add a shortcut

Add a key that starts a program. Put this in your shell file, which is `~/.config/gnoblin/shell.ts` for TypeScript,
`~/.config/gnoblin/shell.py` for Python, or the library crate of a [Rust shell](/rust-shell):

::: tabs

```typescript
gnoblin.input.primary.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
```

```python
from gnoblin import gnoblin


@gnoblin.bind("Super+Return")
def terminal():
    gnoblin.spawn(["foot"])
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let shell = g.clone();
    g.bind("Super+Return", move || shell.spawn("foot"))?;
    Ok(())
}
```

:::

Use an installed terminal. A key is one `bind` call (one decorated function in Python), and your file is the whole
configuration, so keep the stock keys you still want. Volume, media and workspace keys are lines of the starter that
`gnoblinctl config init` writes, in whichever language you chose with `--language`.

To change a key that the file already binds, edit that line: a trigger is bound once, and a second bind of the same
trigger is refused. This is the stock volume-up key with a smaller step:

::: tabs

```typescript
gnoblin.input.primary.bind("XF86AudioRaiseVolume", () => {
    gnoblin.spawn(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "2%+"]);
});
```

```python
from gnoblin import gnoblin


@gnoblin.bind("XF86AudioRaiseVolume")
def volume_up():
    gnoblin.spawn(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "2%+"])
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let shell = g.clone();
    g.bind("XF86AudioRaiseVolume", move || {
        shell.spawn_with("wpctl", ["set-volume", "@DEFAULT_AUDIO_SINK@", "2%+"])
    })?;
    Ok(())
}
```

:::

| Part          | Accepted values                                                         | Effect                                                                                             |
| ------------- | ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| The trigger   | Modifiers and a key joined by `+`: `Super+Return`                       | Runs the function when the key is pressed. See [trigger syntax](/guides/shortcuts#trigger-syntax). |
| The program   | TypeScript and Python: a list, `spawn(["foot"])`. Rust: `spawn("foot")` | Starts the program, passing the arguments, without shell expansion.                                |
| Its arguments | TypeScript and Python: more list items. Rust: `spawn_with`              | Each argument is one string, so spaces inside a string stay in that argument.                      |

Changes apply when you save a TypeScript or Python file, and when you rebuild and reload a Rust shell. To drop a stock key,
delete its line in TypeScript, or its function or `bind` call in the other languages. Use one string per program argument. See
[key names and conflict handling](/guides/shortcuts) for more.
