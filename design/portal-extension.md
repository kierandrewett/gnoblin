# Portal extension design notes

## Goals

- Keep portal support optional for Gnoblin installations that do not need it.
- Keep GTK and Qt out of the Gnoblin core runtime dependencies.
- Let a shell provide its own portal user interface through a small request and response interface.
- Keep stock GNOME and KDE portal backends selectable for users who prefer them.
- Move Gnoblin portal policy and its backend implementation out of the Gnoblin compositor repository into a separate portal repository.
- Rename the GTK implementation and its packages/services from `gnome` to `gtk` where those names identify Gnoblin-owned code. It must not be confused with upstream `xdg-desktop-portal-gnome`.
- Explore a Qt UI implementation for KDE-oriented environments.
- Ship one `gnoblin` package containing the compositor runtime; keep the optional GTK portal backend independently installable.

## Current repository state

- Portal routing is already configurable per interface through `gnoblin.configure.portals`; supported backend IDs include `gtk`, `gnome`, `kde`, and `gnoblin`.
- The session's default `portals.conf` prefers the `gnoblin` backend and falls back to any installed backend. A Lua override can route individual interfaces to stock GNOME, KDE, GTK, or Gnoblin backends.
- `gnoblin-portal` is currently a separate optional package built from the `xdg-desktop-portal-gnome` subproject. It currently has GTK 4 and libadwaita build/runtime dependencies.
- Gnoblin-specific portal sources live under `src/permissions/`. `src/permissions/manifest` overlays policy, identity, file chooser, email, inhibition, and portal service metadata into the upstream GNOME backend source.
- Gnoblin Mutter has corresponding policy and identity overlays. Its native control currently recognizes the backend bus name `org.freedesktop.impl.portal.desktop.gnoblin` for permission and persistent-grant operations.
- The public permissions guide states that Gnoblin Lua permission rules apply only when the Gnoblin portal backend handles the request. They do not control requests routed to a different backend, direct Mutter calls, or every portal interface.
- The compositor and session are packaged as one `gnoblin` runtime. The GTK portal backend remains an optional separate extension.

## Proposed architecture

Keep the standard `xdg-desktop-portal` frontend as the app-facing broker. A Gnoblin backend remains a backend implementation of portal interfaces; it is optional and installed only when requested. Do not replace the standard app-facing portal protocol with a Gnoblin-only protocol.

Split Gnoblin's backend responsibilities into two parts:

1. The portal backend owns app identity, portal request/session lifetime, permission policy, compositor operations, cancellation, and the response returned to the app.
2. A shell UI provider displays user-facing prompts and selection UI, then returns a typed choice to the backend.

The shell-facing interface should describe a UI request and its allowed choices rather than forward raw app D-Bus calls. The backend must keep trusted caller identity and compositor-owned object handles private, bind each UI response to the original request, validate the response, and cancel outstanding UI when the app or request disconnects. A shell UI provider is not the permission authority.

For the initial GTK provider, reuse the existing GTK dialogs behind this interface. This keeps Bingux behavior while making GTK an optional extension dependency. Other shells can implement the same UI interface in their own toolkit. A later Qt provider should implement the identical UI contract, not fork permission logic or duplicate portal policy.

## Backend selection and compatibility

Stock GNOME and KDE portal backends remain valid choices. The existing per-interface routing config can select them explicitly, for example:

```lua
gnoblin.configure {
    portals = {
        default = {"gnome", "kde", "*"},
        interfaces = {
            ["org.freedesktop.impl.portal.ScreenCast"] = {"gnoblin", "*"},
        },
    },
}
```

Routing a request to GNOME or KDE uses that backend's policy and UI; it does not automatically apply Gnoblin's Lua portal rules. Per-interface routing is an intentional escape hatch for users who prefer generic desktop behavior. The extension must not add hard dependencies on either desktop's portal packages.

Do not confuse the `gtk` backend ID (`xdg-desktop-portal-gtk`) with a GTK-based Gnoblin UI provider. The former is a full generic portal backend with its own supported interface set. The latter is only a UI adapter used by Gnoblin's optional backend.

## Naming and repository split

- Give the Gnoblin GTK implementation a `-gtk` name, including the user-facing package and backend metadata. Avoid `-gnome` for Gnoblin-owned artifacts.
- Place the upstream GNOME backend fork and Gnoblin-owned policy/identity/portal changes in a separate repository dedicated to portal implementations. Preserve upstream attribution and keep the upstream delta reviewable.
- Keep only session routing defaults and generic integration points in the Gnoblin core repository.
- Update Mutter's permission integration to consume a stable Gnoblin-owned contract without requiring permission-policy source to be duplicated in the compositor repository. The current hard-coded Gnoblin portal bus name and grant-control calls need an explicit compatibility plan before the repo split.
- A Qt implementation is a future adapter to explore. It should share the same policy and backend contract; it must not become a required KDE dependency for Gnoblin core or the GTK extension.

## Package consolidation

The user-facing install requires one `gnoblin` package that contains the matching compositor runtime and session files. Keep any development headers/pkg-config files as a separate development package if build consumers need them. The optional portal extension remains independently installable.

Before changing package names or repository ownership, update the manifest generator, source release archives, build scripts, distro specs, session installer, install scripts, package isolation checks, and upgrade/obsolete rules together. Preserve a package upgrade path from the existing split packages.

## Decisions still needed during implementation

- Exact new portal repository name and GitHub owner/remote.
- Whether the default Gnoblin session routes to the Gnoblin extension when installed and falls back to generic backends when it is absent; the current session config already behaves this way.
- Exact shell UI provider transport and schema. Prefer a small D-Bus contract with typed request kinds, opaque request IDs, cancellation, and validated structured responses.
- Which portal interfaces are Gnoblin-owned versus delegated to the existing `gtk`, GNOME, or KDE implementations.
- Whether generic GNOME/KDE fallback is a build/package dependency (it should not be) or a user-selected route.
- The versioning and migration policy for upgrading systems from the former split runtime packages to the unified `gnoblin` runtime package.
