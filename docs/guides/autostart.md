# Autostart

A spawn call at the top level of the shell's entry file (`shell.ts`, `shell.py`, or the `main` of a Rust shell) starts its program when the shell loads the file. A reload does not start
a second copy: the program that is running stays, and one you take out of the file is stopped (see
[processes](/shell-api/processes#programs-the-file-starts)).

The programs end with the shell, so this is not a startup list for the login session. For that, use one of these:

- A systemd user unit, enabled with `systemctl --user enable --now waybar.service`, for a program that should run for the
  whole login session.
- The session's own startup, such as the `exec` lines of your compositor wrapper or a Bingux module, which starts its shell
  and layer clients when the session begins.
- A key that starts the program on demand, which is a `bind` call:

::: tabs

```typescript
import { gnoblin } from "gnoblin-core/generated/objects";

gnoblin.input.primary.bind("Super+b", () => {
    gnoblin.spawn(["waybar"]);
});
```

```python
from gnoblin import gnoblin


@gnoblin.bind("Super+b")
def start_waybar(event):
    gnoblin.spawn(["waybar"])
```

```rust
use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    g.bind("Super+b", move || {
        if let Err(error) = g.spawn("waybar") {
            g.log(&format!("cannot start waybar: {error}"));
        }
    })?;
    Ok(())
}
```

:::

Gnoblin runs a command directly, without shell expansion. Pass the program and then each argument as a separate string.
If you need pipes or redirection, explicitly run a shell. See [command syntax](/guides/shortcuts#commands-and-shell-syntax).

Gnoblin does not read XDG autostart `.desktop` files. A program that starts gnome-keyring takes `org.freedesktop.secrets`
from Gnoblin's keyring; see [Apps reach gnome-keyring instead](/config/configure/keyring#apps-reach-gnome-keyring-instead).

See [files and load order](/guides/files_and_load_order) for how a reload runs your files.
