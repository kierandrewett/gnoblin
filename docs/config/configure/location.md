# Location settings

Gnoblin registers as the session's GeoClue agent, which holds the global location policy: whether location requests may
be approved and the maximum precision an approved request gets. The policy has no TypeScript home yet: see
[settings with no TypeScript home yet](/js-settings#dropped-settings).

The agent follows the system's
`org.gnome.system.location` settings, and the
[schema](https://github.com/GNOME/gsettings-desktop-schemas/blob/main/schemas/org.gnome.system.location.gschema.xml.in)
names them. The agent does not approve individual applications. GeoClue asks about an app only when it can identify
it, as [below](#when-geoclue-asks-the-agent).

## When GeoClue asks the agent

GeoClue asks the agent about an app only when it can identify the app as a sandboxed Flatpak app. It reads that
identity from the app's systemd scope (`app-flatpak-ID-N.scope`). Every other app counts as a system component:
GeoClue does not ask, and the app receives location up to the policy's accuracy cap.

The policy applies to every app, with or without a request.

## Allow the agent in GeoClue

GeoClue accepts only the agents its `[agent]` `whitelist` names. Gnoblin registers as `gnoblin`, so the system needs a
drop-in that keeps GeoClue's stock agents and adds it. `data/geoclue/50-gnoblin.conf` in the source tree is that drop-in:
copy it to `/etc/geoclue/conf.d/50-gnoblin.conf`.

```ini
[agent]
whitelist=geoclue-demo-agent;gnome-shell;io.elementary.desktop.agent-geoclue2;sm.puri.Phosh;lipstick;gnoblin
```

GeoClue applies a `conf.d` value as an override, so the line repeats the stock identifiers.

## Stop the GeoClue demo agent

GeoClue accepts one agent for each user. On Fedora the `geoclue-demo-agent`
autostart entry hides itself only in GNOME, so it also starts in a Gnoblin
session. It approves every request, and it can take the agent slot before
Gnoblin registers.

To stop it, create `~/.config/autostart/geoclue-demo-agent.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=Geoclue Demo agent
Exec=/usr/libexec/geoclue-2.0/demos/agent
Hidden=true
```

Run `systemctl --user daemon-reload`, then log out and in. A running demo agent
keeps running until then. To check that GeoClue accepted Gnoblin, stop the
service with `sudo systemctl stop geoclue`, start it by hand with
`sudo -u geoclue env G_MESSAGES_DEBUG=Geoclue /usr/libexec/geoclue`, and look
for `New agent for user ID` in its output.
