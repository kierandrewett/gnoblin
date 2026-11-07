# Capture protocols

Screen capture in Gnoblin is a privileged capability. This note records how
capture is exposed today, what that exposure really grants, and what is still
undecided.

## Current model

- `zwlr_screencopy_manager_v1` is advertised only on the privileged socket,
  `$XDG_RUNTIME_DIR/gnoblin-wayland-privileged.sock`. The ordinary socket and
  sandboxed clients never see it.
- The privileged socket is created with mode `0600`. That limits connections
  to the owning UID. It does **not** limit them to capture tools.
- Any process running as that user that finds the socket path gets every
  privileged global: screencopy, image copy capture, data-control (clipboard
  read and write), layer-shell, foreign-toplevel, output management and power,
  gamma control, and the gnoblin frame and session-host globals. A same-user
  program can read the screen and the clipboard without being a capture tool.
- The session exports the socket name as `GNOBLIN_PRIVILEGED_DISPLAY` to every
  child its guardian starts: the compositor, services, the runtime, the front
  and autostart entries. Their descendants inherit it. D-Bus-activated services
  do not get it in this build, because the activation environment export is
  logged but not applied. The name is fixed, so the variable is a pointer, not
  a grant.
- A user runs a capture tool explicitly:

    ```sh
    WAYLAND_DISPLAY=$GNOBLIN_PRIVILEGED_DISPLAY grim out.png
    ```

- Sandbox reach, checked on this machine:
    - A bubblewrap sandbox with a tmpfs over `$XDG_RUNTIME_DIR` and only the
      Wayland socket bound hides the privileged socket. Verified.
    - Flatpak 1.16.6 with the default permissions of `app.drey.Warp` hides it.
      Verified for that one app.
    - Other Flatpak apps, apps granted `xdg-run` or a runtime-directory bind, and
      the session's own sandboxes are **unverified**. Any sandbox that exposes
      `$XDG_RUNTIME_DIR` can reach the socket.
- Screen sharing through the desktop portal is a separate path. It depends on
  the selected portal backend and does not use screencopy on the privileged
  socket.

## Why this shape

An ordinary-socket application that could copy another client's pixels could
read every window, including password fields. The privileged socket keeps that
power off the ordinary socket and out of sandboxes that do not expose the path.
It does not keep it from other programs the same user runs.

## Needs Kieran

- Should grim-style tools get capture through a portal-style consent prompt
  instead of the privileged socket? The consent path would ask the user each
  time a tool captures, and would fit sandboxed tools that the user has not
  declared in autostart. The trade-off is a prompt for each screenshot against
  a single, user-declared trust decision. This is the only option that stops
  an arbitrary same-user process from capturing without asking.
- Who should use the privileged socket? Today the answer is "tools the user
  declares in autostart, or runs by hand", but the socket does not enforce that
  answer. Decide whether that list stays informal or is written into the config.
- Should `GNOBLIN_PRIVILEGED_DISPLAY` reach every child, or only autostart
  entries and the front? Today every child gets it. I recommend autostart and
  the front only, since internal roles do not need the name. This is a tidiness
  choice, not a security boundary: the path is fixed, so any same-user process
  can find the socket either way. Decide which scope to keep.
