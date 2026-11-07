# Capture protocols

Screen capture in Gnoblin is a privileged capability. This note records how
capture is exposed today and what is still undecided.

## Current model

- `zwlr_screencopy_manager_v1` is advertised only on the privileged socket,
  `$XDG_RUNTIME_DIR/gnoblin-wayland-privileged.sock`. The ordinary socket and
  sandboxed clients never see it.
- The privileged socket is created with mode `0600`. Only the owning user can
  connect, and the compositor refuses the privileged globals to any other
  client.
- The session exports the socket name as `GNOBLIN_PRIVILEGED_DISPLAY` to the
  programs it starts. A user runs a capture tool explicitly:

    ```sh
    WAYLAND_DISPLAY=$GNOBLIN_PRIVILEGED_DISPLAY grim out.png
    ```

- Autostart entries are declared by the user, so the user has already chosen to
  trust them with the privileged socket. Nothing else gains capture by name,
  process or uid.
- Screen sharing through the desktop portal is a separate path. It depends on
  the selected portal backend and does not use screencopy on the privileged
  socket.

## Why this shape

An application on the ordinary socket that could copy another client's pixels
would be able to read every window, including password fields and other apps'
private content. The privileged socket keeps that power out of reach of
applications and sandboxes while still letting the user's own tools capture the
screen.

## Needs Kieran

- Should grim-style tools get capture through a portal-style consent prompt
  instead of the privileged socket? The consent path would ask the user each
  time a tool captures, and would fit sandboxed tools that the user has not
  declared in autostart. The trade-off is a prompt for each screenshot against
  a single, user-declared trust decision.
- Who should use the privileged socket? Today the answer is "tools the user
  declares in autostart, or runs by hand". Decide whether that list stays
  informal or is written into the config.
