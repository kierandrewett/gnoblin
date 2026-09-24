# gnoblin.configure.input

Configure this part of `gnoblin.configure` with the `input` key.

Input settings go in `gnoblin.configure {input = {...}}` and apply on reload.
Each group is optional. Fields you omit continue to use the corresponding
GNOME/Mutter setting. Open a group for its fields and valid values:

- [`gnoblin.configure.input.mouse`](/config/configure/input/mouse)
- [`gnoblin.configure.input.touchpad`](/config/configure/input/touchpad)
- [`gnoblin.configure.input.keyboard`](/config/configure/input/keyboard)
- [`gnoblin.configure.input.tablets`](/config/configure/input/tablets)
- [`gnoblin.configure.input.styluses`](/config/configure/input/styluses)
- [`gnoblin.configure.input.orientation_lock`](/config/configure/input/orientation_lock)

`numlock_state` is kept in memory while Gnoblin's config is active. Device
overrides apply only to the tablet or pen IDs listed in their group. Removing
an override restores the corresponding system setting.
