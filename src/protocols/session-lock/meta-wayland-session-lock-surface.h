/* ext-session-lock-v1 surface role for Gnoblin Mutter. */

#pragma once

#include "wayland/meta-wayland-shell-surface.h"

typedef struct _MetaWaylandSessionLockSurface MetaWaylandSessionLockSurface;

#define META_TYPE_WAYLAND_SESSION_LOCK_SURFACE \
  (meta_wayland_session_lock_surface_get_type ())
G_DECLARE_FINAL_TYPE (MetaWaylandSessionLockSurface,
                      meta_wayland_session_lock_surface,
                      META, WAYLAND_SESSION_LOCK_SURFACE,
                      MetaWaylandShellSurface)

MetaWaylandSessionLockSurface *
meta_wayland_session_lock_surface_new (MetaWaylandSurface *surface,
                                       struct wl_client   *client,
                                       uint32_t            id,
                                       struct wl_resource *output_resource,
                                       gpointer             owner);

struct wl_resource *
meta_wayland_session_lock_surface_get_resource (MetaWaylandSessionLockSurface *surface);

MetaWaylandOutput *
meta_wayland_session_lock_surface_get_output (MetaWaylandSessionLockSurface *surface);

/* Input admission is allowed only after the lock role has attached an
 * acknowledged buffer.  These accessors deliberately expose no mutable
 * state, so the controller can verify its own output map before dispatch. */
MetaWaylandSurface *
meta_wayland_session_lock_surface_get_wayland_surface (MetaWaylandSessionLockSurface *surface);

gboolean
meta_wayland_session_lock_surface_is_mapped (MetaWaylandSessionLockSurface *surface);

void meta_wayland_session_lock_surface_send_configure (MetaWaylandSessionLockSurface *surface);
void meta_wayland_session_lock_surface_close (MetaWaylandSessionLockSurface *surface);

/* Controller callback used by the role when its Wayland object disappears. */
void meta_wayland_session_lock_surface_destroyed (gpointer owner,
                                                  MetaWaylandSessionLockSurface *surface);
