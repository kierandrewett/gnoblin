/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "config.h"

#include "wayland/meta-wayland-blur-fade.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-surface-private.h"
#include "wayland/gnoblin-config.h"
#include "meta/window.h"
#include "gnoblin-blur-fade-v1-server-protocol.h"

#define FADE_KEY "gnoblin-buffer-fades"
#define MAX_FADES 32

static void
destroy_manager (struct wl_client *client,
                 struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static void
set_fades (struct wl_client *client,
           struct wl_resource *resource,
           struct wl_resource *surface_resource,
           struct wl_array *fades)
{
  MetaWaylandSurface *surface = wl_resource_get_user_data (surface_resource);
  MetaWaylandSurfaceState *pending = meta_wayland_surface_get_pending_state (surface);
  const int32_t *values = fades->data;
  gboolean valid = fades->size % (5 * sizeof (int32_t)) == 0 &&
                   fades->size <= MAX_FADES * 5 * sizeof (int32_t);

  for (size_t i = 0; valid && i < fades->size / sizeof (int32_t); i += 5)
    {
      for (size_t j = 0; j < 4; j++)
        valid = valid && values[i + j] >= -65536 * 256 && values[i + j] <= 65536 * 256;
      valid = valid && values[i + 2] >= 0 && values[i + 3] >= 0 &&
              values[i + 4] >= 0 && values[i + 4] <= 256;
    }
  if (!valid)
    {
      wl_resource_post_error (resource, GNOBLIN_BLUR_FADE_MANAGER_V1_ERROR_INVALID_FADES,
                              "Expected at most 32 fixed-point fade rectangles");
      return;
    }
  g_object_set_data_full (G_OBJECT (pending), FADE_KEY,
                          g_bytes_new (fades->data, fades->size), (GDestroyNotify) g_bytes_unref);
}

static const struct gnoblin_blur_fade_manager_v1_interface implementation = {
  destroy_manager,
  set_fades,
};

static void
bind_manager (struct wl_client *client,
              void *data,
              uint32_t version,
              uint32_t id)
{
  struct wl_resource *resource = wl_resource_create (client,
    &gnoblin_blur_fade_manager_v1_interface, 1, id);
  wl_resource_set_implementation (resource, &implementation, NULL, NULL);
}

void
meta_wayland_init_blur_fade (MetaWaylandCompositor *compositor)
{
  if (!gnoblin_config_protocol_enabled ("blur-fade"))
    return;
  if (!wl_global_create (compositor->wayland_display,
                        &gnoblin_blur_fade_manager_v1_interface, 1, NULL, bind_manager))
    g_error ("Failed to register blur fade metadata");
}

void
meta_wayland_blur_fade_merge_state (MetaWaylandSurfaceState *from,
                                   MetaWaylandSurfaceState *to)
{
  GBytes *fades = g_object_steal_data (G_OBJECT (from), FADE_KEY);
  if (fades)
    g_object_set_data_full (G_OBJECT (to), FADE_KEY, fades, (GDestroyNotify) g_bytes_unref);
}

void
meta_wayland_blur_fade_apply_state (MetaWaylandSurface *surface,
                                   MetaWaylandSurfaceState *state)
{
  GBytes *fades = g_object_get_data (G_OBJECT (state), FADE_KEY);
  MetaWindow *window;
  ClutterActor *actor;

  if (fades)
    g_object_set_data_full (G_OBJECT (surface), FADE_KEY,
                            g_bytes_ref (fades), (GDestroyNotify) g_bytes_unref);
  fades = g_object_get_data (G_OBJECT (surface), FADE_KEY);
  window = meta_wayland_surface_get_window (surface);
  if (!window || !fades || g_object_get_data (G_OBJECT (window), FADE_KEY) == fades)
    return;
  g_object_set_data_full (G_OBJECT (window), FADE_KEY,
                          g_bytes_ref (fades), (GDestroyNotify) g_bytes_unref);
  actor = CLUTTER_ACTOR (meta_window_get_compositor_private (window));
  if (actor)
    clutter_actor_queue_redraw (actor);
}
