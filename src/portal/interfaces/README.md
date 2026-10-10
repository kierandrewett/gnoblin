# Bundled desktop-portal interface XML

These files are exact copies of the `data/org.freedesktop.impl.portal.*.xml`
files from the `flatpak/xdg-desktop-portal` upstream tag `1.21.1`, commit
`ced43a7c1c61083fac012a7c7991482ed9798fa3`.

They are distributed under the upstream project's LGPL-2.1-or-later license.
Gnoblin copies them into the pinned portal backend's `data/interfaces/` at build
time. The backend uses them only as GDBus code-generation input, allowing its
newer additive methods to compile against an installed `xdg-desktop-portal`
1.20.x frontend. Runtime method negotiation remains the frontend's responsibility.
