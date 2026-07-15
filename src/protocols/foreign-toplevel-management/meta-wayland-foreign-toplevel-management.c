/*
 * gnoblin: wlr-foreign-toplevel-management-unstable-v1 support for mutter.
 *
 * Exposes mutter's normal toplevel windows to clients such as taskbars and
 * window switchers (waybar, wlrctl) and lets them drive window management:
 * activate, close, (un)minimize, (un)maximize and (un)fullscreen, plus live
 * title/app_id/state updates. The read-only ext-foreign-toplevel-list protocol
 * covers pure enumeration; this protocol adds control.
 *
 * Note: the optional output_enter/output_leave events are not emitted. Mapping
 * a MetaWindow to its per-client wl_output resources requires the compositor's
 * private output table; clients that rely on these events simply treat the
 * toplevel's output as unknown, which is protocol-valid. This can be added
 * later if a consumer needs per-output toplevel filtering.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-foreign-toplevel-management.h"
#include "wayland/meta-wayland-foreign-toplevel-common.h"

#include <gio/gio.h>
#include <wayland-server.h>

#include "meta/display.h"
#include "meta/meta-context.h"
#include "meta/window.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/gnoblin-config.h"

#include "wlr-foreign-toplevel-management-unstable-v1-server-protocol.h"

#define META_WLR_FOREIGN_TOPLEVEL_MANAGEMENT_VERSION 3

typedef struct _MetaWaylandForeignToplevelManager
{
  struct wl_resource *resource;
  MetaDisplay *display;
  gulong window_created_id;
  gboolean stopped;
  GHashTable *handles; /* MetaWindow* -> MetaWaylandForeignToplevelHandle* */
} MetaWaylandForeignToplevelManager;

typedef struct _MetaWaylandForeignToplevelHandle
{
  struct wl_resource *resource;
  MetaWaylandForeignToplevelManager *manager; /* NULL once manager gone */
  MetaWindow *window;                         /* NULL once unmanaged */
  gulong unmanaged_id;
  gulong notify_title_id;
  gulong notify_wm_class_id;
  gulong notify_gtk_app_id_id;
  gulong notify_minimized_id;
  gulong notify_maximized_h_id;
  gulong notify_maximized_v_id;
  gulong notify_fullscreen_id;
  gulong notify_focus_id;
} MetaWaylandForeignToplevelHandle;


static guint32
current_time (MetaWaylandForeignToplevelHandle *handle)
{
  MetaDisplay *display = handle->manager->display;

  return meta_display_get_current_time_roundtrip (display);
}

static void
handle_send_title (MetaWaylandForeignToplevelHandle *handle)
{
  const char *title = meta_window_get_title (handle->window);

  zwlr_foreign_toplevel_handle_v1_send_title (handle->resource,
                                              title ? title : "");
}

static void
handle_send_app_id (MetaWaylandForeignToplevelHandle *handle)
{
  const char *app_id =
    meta_gnoblin_foreign_toplevel_window_app_id (handle->window);

  zwlr_foreign_toplevel_handle_v1_send_app_id (handle->resource, app_id);
}

static void
handle_send_state (MetaWaylandForeignToplevelHandle *handle)
{
  struct wl_array states;
  gboolean minimized = FALSE;
  uint32_t *entry;

  wl_array_init (&states);

  g_object_get (handle->window, "minimized", &minimized, NULL);

  if (meta_window_get_maximize_flags (handle->window) == META_MAXIMIZE_BOTH)
    {
      entry = wl_array_add (&states, sizeof *entry);
      *entry = ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_STATE_MAXIMIZED;
    }
  if (minimized)
    {
      entry = wl_array_add (&states, sizeof *entry);
      *entry = ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_STATE_MINIMIZED;
    }
  if (meta_window_has_focus (handle->window))
    {
      entry = wl_array_add (&states, sizeof *entry);
      *entry = ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_STATE_ACTIVATED;
    }
  if (meta_window_is_fullscreen (handle->window))
    {
      entry = wl_array_add (&states, sizeof *entry);
      *entry = ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_STATE_FULLSCREEN;
    }

  zwlr_foreign_toplevel_handle_v1_send_state (handle->resource, &states);
  wl_array_release (&states);
}

static void
on_notify_title (GObject *o, GParamSpec *p, gpointer user_data)
{
  MetaWaylandForeignToplevelHandle *handle = user_data;

  if (!handle->window)
    return;
  handle_send_title (handle);
  zwlr_foreign_toplevel_handle_v1_send_done (handle->resource);
}

static void
on_notify_app_id (GObject *o, GParamSpec *p, gpointer user_data)
{
  MetaWaylandForeignToplevelHandle *handle = user_data;

  if (!handle->window)
    return;
  handle_send_app_id (handle);
  zwlr_foreign_toplevel_handle_v1_send_done (handle->resource);
}

static void
on_notify_state (GObject *o, GParamSpec *p, gpointer user_data)
{
  MetaWaylandForeignToplevelHandle *handle = user_data;

  if (!handle->window)
    return;
  handle_send_state (handle);
  zwlr_foreign_toplevel_handle_v1_send_done (handle->resource);
}

static void
handle_disconnect_window (MetaWaylandForeignToplevelHandle *handle)
{
  if (!handle->window)
    return;

  g_clear_signal_handler (&handle->unmanaged_id, handle->window);
  g_clear_signal_handler (&handle->notify_title_id, handle->window);
  g_clear_signal_handler (&handle->notify_wm_class_id, handle->window);
  g_clear_signal_handler (&handle->notify_gtk_app_id_id, handle->window);
  g_clear_signal_handler (&handle->notify_minimized_id, handle->window);
  g_clear_signal_handler (&handle->notify_maximized_h_id, handle->window);
  g_clear_signal_handler (&handle->notify_maximized_v_id, handle->window);
  g_clear_signal_handler (&handle->notify_fullscreen_id, handle->window);
  g_clear_signal_handler (&handle->notify_focus_id, handle->window);

  if (handle->manager)
    g_hash_table_remove (handle->manager->handles, handle->window);
  handle->window = NULL;
}

static void
on_window_unmanaged (MetaWindow *window, gpointer user_data)
{
  MetaWaylandForeignToplevelHandle *handle = user_data;

  handle_disconnect_window (handle);
  zwlr_foreign_toplevel_handle_v1_send_closed (handle->resource);
}

/* requests */

static void
handle_set_maximized (struct wl_client *c, struct wl_resource *resource)
{
  MetaWaylandForeignToplevelHandle *handle = wl_resource_get_user_data (resource);
  if (handle->window)
    meta_window_maximize (handle->window);
}

static void
handle_unset_maximized (struct wl_client *c, struct wl_resource *resource)
{
  MetaWaylandForeignToplevelHandle *handle = wl_resource_get_user_data (resource);
  if (handle->window)
    meta_window_unmaximize (handle->window);
}

static void
handle_set_minimized (struct wl_client *c, struct wl_resource *resource)
{
  MetaWaylandForeignToplevelHandle *handle = wl_resource_get_user_data (resource);
  if (handle->window)
    meta_window_minimize (handle->window);
}

static void
handle_unset_minimized (struct wl_client *c, struct wl_resource *resource)
{
  MetaWaylandForeignToplevelHandle *handle = wl_resource_get_user_data (resource);
  if (handle->window)
    meta_window_unminimize (handle->window);
}

static void
handle_activate (struct wl_client   *c,
                 struct wl_resource *resource,
                 struct wl_resource *seat_resource)
{
  MetaWaylandForeignToplevelHandle *handle = wl_resource_get_user_data (resource);
  if (handle->window)
    meta_window_activate (handle->window, current_time (handle));
}

static void
handle_close (struct wl_client *c, struct wl_resource *resource)
{
  MetaWaylandForeignToplevelHandle *handle = wl_resource_get_user_data (resource);
  if (handle->window)
    meta_window_delete (handle->window, current_time (handle));
}

static void
handle_set_rectangle (struct wl_client   *c,
                      struct wl_resource *resource,
                      struct wl_resource *surface,
                      int32_t x, int32_t y, int32_t width, int32_t height)
{
  /* Hint for minimize/restore animations; mutter does not consume it. */
}

static void
handle_set_fullscreen (struct wl_client   *c,
                       struct wl_resource *resource,
                       struct wl_resource *output)
{
  MetaWaylandForeignToplevelHandle *handle = wl_resource_get_user_data (resource);
  if (handle->window)
    meta_window_make_fullscreen (handle->window);
}

static void
handle_unset_fullscreen (struct wl_client *c, struct wl_resource *resource)
{
  MetaWaylandForeignToplevelHandle *handle = wl_resource_get_user_data (resource);
  if (handle->window)
    meta_window_unmake_fullscreen (handle->window);
}

static void
handle_handle_destroy (struct wl_client *c, struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct zwlr_foreign_toplevel_handle_v1_interface handle_interface = {
  handle_set_maximized,
  handle_unset_maximized,
  handle_set_minimized,
  handle_unset_minimized,
  handle_activate,
  handle_close,
  handle_set_rectangle,
  handle_handle_destroy,
  handle_set_fullscreen,
  handle_unset_fullscreen,
};

static void
handle_destroy (struct wl_resource *resource)
{
  MetaWaylandForeignToplevelHandle *handle = wl_resource_get_user_data (resource);

  handle_disconnect_window (handle);
  g_free (handle);
}

static void
toplevel_management_advertise (MetaWaylandForeignToplevelManager *manager,
                               MetaWindow                        *window)
{
  struct wl_client *client = wl_resource_get_client (manager->resource);
  MetaWaylandForeignToplevelHandle *handle;
  struct wl_resource *resource;
  MetaWindow *parent;

  resource = wl_resource_create (client,
                                 &zwlr_foreign_toplevel_handle_v1_interface,
                                 wl_resource_get_version (manager->resource), 0);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  handle = g_new0 (MetaWaylandForeignToplevelHandle, 1);
  handle->resource = resource;
  handle->manager = manager;
  handle->window = window;
  wl_resource_set_implementation (resource, &handle_interface, handle,
                                  handle_destroy);

  g_hash_table_insert (manager->handles, window, handle);

  zwlr_foreign_toplevel_manager_v1_send_toplevel (manager->resource, resource);

  handle_send_title (handle);
  handle_send_app_id (handle);
  handle_send_state (handle);

  if (wl_resource_get_version (resource) >=
      ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_PARENT_SINCE_VERSION)
    {
      MetaWaylandForeignToplevelHandle *parent_handle = NULL;

      parent = meta_window_get_transient_for (window);
      if (parent)
        parent_handle = g_hash_table_lookup (manager->handles, parent);

      zwlr_foreign_toplevel_handle_v1_send_parent (
        resource, parent_handle ? parent_handle->resource : NULL);
    }

  zwlr_foreign_toplevel_handle_v1_send_done (resource);

  handle->unmanaged_id =
    g_signal_connect (window, "unmanaged", G_CALLBACK (on_window_unmanaged), handle);
  handle->notify_title_id =
    g_signal_connect (window, "notify::title", G_CALLBACK (on_notify_title), handle);
  handle->notify_wm_class_id =
    g_signal_connect (window, "notify::wm-class", G_CALLBACK (on_notify_app_id), handle);
  handle->notify_gtk_app_id_id =
    g_signal_connect (window, "notify::gtk-application-id", G_CALLBACK (on_notify_app_id), handle);
  handle->notify_minimized_id =
    g_signal_connect (window, "notify::minimized", G_CALLBACK (on_notify_state), handle);
  handle->notify_maximized_h_id =
    g_signal_connect (window, "notify::maximized-horizontally", G_CALLBACK (on_notify_state), handle);
  handle->notify_maximized_v_id =
    g_signal_connect (window, "notify::maximized-vertically", G_CALLBACK (on_notify_state), handle);
  handle->notify_fullscreen_id =
    g_signal_connect (window, "notify::fullscreen", G_CALLBACK (on_notify_state), handle);
  handle->notify_focus_id =
    g_signal_connect (window, "notify::appears-focused", G_CALLBACK (on_notify_state), handle);
}

static void
on_window_created (MetaDisplay *display, MetaWindow *window, gpointer user_data)
{
  MetaWaylandForeignToplevelManager *manager = user_data;

  if (manager->stopped)
    return;
  if (!meta_gnoblin_foreign_toplevel_window_is_exposable (window))
    return;

  toplevel_management_advertise (manager, window);
}

static void
manager_handle_stop (struct wl_client *c, struct wl_resource *resource)
{
  MetaWaylandForeignToplevelManager *manager = wl_resource_get_user_data (resource);

  if (manager->stopped)
    return;

  manager->stopped = TRUE;
  g_clear_signal_handler (&manager->window_created_id, manager->display);
  zwlr_foreign_toplevel_manager_v1_send_finished (resource);
}

static const struct zwlr_foreign_toplevel_manager_v1_interface manager_interface = {
  manager_handle_stop,
};

static void
manager_destroy (struct wl_resource *resource)
{
  MetaWaylandForeignToplevelManager *manager = wl_resource_get_user_data (resource);
  GList *handles, *l;

  g_clear_signal_handler (&manager->window_created_id, manager->display);

  /* Detach surviving handles so their destructors do not touch freed state. */
  handles = g_hash_table_get_values (manager->handles);
  for (l = handles; l; l = l->next)
    {
      MetaWaylandForeignToplevelHandle *handle = l->data;
      handle->manager = NULL;
    }
  g_list_free (handles);
  g_hash_table_destroy (manager->handles);

  g_free (manager);
}

static void
bind_foreign_toplevel_manager (struct wl_client *client,
                               void             *data,
                               uint32_t          version,
                               uint32_t          id)
{
  MetaWaylandCompositor *compositor = data;
  MetaContext *context = meta_wayland_compositor_get_context (compositor);
  MetaDisplay *display = meta_context_get_display (context);
  MetaWaylandForeignToplevelManager *manager;
  struct wl_resource *resource;
  GList *windows, *l;

  resource = wl_resource_create (client,
                                 &zwlr_foreign_toplevel_manager_v1_interface,
                                 version, id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  manager = g_new0 (MetaWaylandForeignToplevelManager, 1);
  manager->resource = resource;
  manager->display = display;
  manager->handles = g_hash_table_new (NULL, NULL);
  wl_resource_set_implementation (resource, &manager_interface, manager,
                                  manager_destroy);

  manager->window_created_id =
    g_signal_connect (display, "window-created",
                      G_CALLBACK (on_window_created), manager);

  windows = meta_display_list_all_windows (display);
  for (l = windows; l; l = l->next)
    {
      MetaWindow *window = l->data;

      if (meta_gnoblin_foreign_toplevel_window_is_exposable (window))
        toplevel_management_advertise (manager, window);
    }
  g_list_free (windows);
}

void
meta_wayland_init_foreign_toplevel_management (MetaWaylandCompositor *compositor)
{
  if (!gnoblin_config_protocol_enabled ("wlr-foreign-toplevel-management"))
    {
      g_message ("Gnoblin wlr-foreign-toplevel-management protocol disabled by settings");
      return;
    }

  if (!wl_global_create (compositor->wayland_display,
                         &zwlr_foreign_toplevel_manager_v1_interface,
                         META_WLR_FOREIGN_TOPLEVEL_MANAGEMENT_VERSION,
                         compositor,
                         bind_foreign_toplevel_manager))
    g_error ("Failed to register wlr-foreign-toplevel-management global");
}
