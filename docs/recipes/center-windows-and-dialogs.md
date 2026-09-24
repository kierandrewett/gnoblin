# Center windows and attach dialogs

Center new windows and keep modal dialogs with their parent window:

```lua
gnoblin.configure {
    window_management = {
        center_new_windows = true,
        attach_modal_dialogs = true,
    },
}
```

These settings apply to Gnoblin's window placement policy. See the
[window-management reference](/config/configure/window_management).
