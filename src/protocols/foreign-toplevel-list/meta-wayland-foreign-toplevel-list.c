/*
 * gnoblin: ext-foreign-toplevel-list-v1 support for mutter.
 *
 * Implements the ext-foreign-toplevel-list-v1 protocol by exposing mutter's
 * normal toplevel windows (MetaWindow of type META_WINDOW_NORMAL) to clients
 * such as taskbars and window switchers (waybar, lswt). This is read-only:
 * each toplevel handle carries a stable identifier, title and app_id, and is
 * closed when the window is unmanaged. Window control (activate/close/minimize)
 * belongs to the separate wlr-foreign-toplevel-management protocol.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-foreign-toplevel-list.h"

#include <gio/gio.h>
#include <wayland-server.h>

#include "meta/display.h"
#include "meta/meta-context.h"
#include "meta/window.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/gnoblin-config.h"

#include "ext-foreign-toplevel-list-v1-server-protocol.h"

#define META_EXT_FOREIGN_TOPLEVEL_LIST_VERSION 1

typedef struct _MetaWaylandForeignToplevelList
{
  struct wl_resource *resource;
  MetaDisplay *display;
  gulong window_created_id;
  gboolean stopped;
  GList *handles; /* MetaWaylandForeignToplevelHandle* (not owned) */
} MetaWaylandForeignToplevelList;

typedef struct _MetaWaylandForeignToplevelHandle
{
  struct wl_resource *resource;
  MetaWaylandForeignToplevelList *list; /* NULL once the manager is gone */
  MetaWindow *window;                   /* NULL once unmanaged */
  gulong unmanaged_id;
  gulong notify_title_id;
  gulong notify_wm_class_id;
  gulong notify_gtk_app_id_id;
} MetaWaylandForeignToplevelHandle;

static gboolean
window_is_exposable (MetaWindow *window)
{
  return meta_window_get_window_type (window) == META_WINDOW_NORMAL;
}

static const char *
window_app_id (MetaWindow *window)
{
  const char *app_id;

  app_id = meta_window_get_sandboxed_app_id (window);
  if (app_id)
    return app_id;
  app_id = meta_window_get_gtk_application_id (window);
  if (app_id)
    return app_id;
  app_id = meta_window_get_wm_class (window);
  if (app_id)
    return app_id;

  return "";
}

static void
handle_send_title (MetaWaylandForeignToplevelHandle *handle)
{
  const char *title = meta_window_get_title (handle->window);

  ext_foreign_toplevel_handle_v1_send_title (handle->resource,
                                             title ? title : "");
}

static void
handle_send_app_id (MetaWaylandForeignToplevelHandle *handle)
{
  ext_foreign_toplevel_handle_v1_send_app_id (handle->resource,
                                              window_app_id (handle->window));
}

static void
on_window_notify_title (GObject    *object,
                        GParamSpec *pspec,
                        gpointer    user_data)
{
  MetaWaylandForeignToplevelHandle *handle = user_data;

  if (!handle->window)
    return;

  handle_send_title (handle);
  ext_foreign_toplevel_handle_v1_send_done (handle->resource);
}

static void
on_window_notify_app_id (GObject    *object,
                         GParamSpec *pspec,
                         gpointer    user_data)
{
  MetaWaylandForeignToplevelHandle *handle = user_data;

  if (!handle->window)
    return;

  handle_send_app_id (handle);
  ext_foreign_toplevel_handle_v1_send_done (handle->resource);
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
  handle->window = NULL;
}

static void
on_window_unmanaged (MetaWindow *window,
                     gpointer    user_data)
{
  MetaWaylandForeignToplevelHandle *handle = user_data;

  handle_disconnect_window (handle);
  ext_foreign_toplevel_handle_v1_send_closed (handle->resource);
}

static void
handle_destroy (struct wl_resource *resource)
{
  MetaWaylandForeignToplevelHandle *handle =
    wl_resource_get_user_data (resource);

  handle_disconnect_window (handle);
  if (handle->list)
    handle->list->handles = g_list_remove (handle->list->handles, handle);

  g_free (handle);
}

static void
handle_handle_destroy (struct wl_client   *client,
                       struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_foreign_toplevel_handle_v1_interface handle_interface = {
  handle_handle_destroy,
};

static void
foreign_toplevel_list_advertise (MetaWaylandForeignToplevelList *list,
                                 MetaWindow                     *window)
{
  struct wl_client *client = wl_resource_get_client (list->resource);
  MetaWaylandForeignToplevelHandle *handle;
  struct wl_resource *resource;
  g_autofree char *identifier = NULL;

  resource = wl_resource_create (client,
                                 &ext_foreign_toplevel_handle_v1_interface,
                                 wl_resource_get_version (list->resource),
                                 0);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  handle = g_new0 (MetaWaylandForeignToplevelHandle, 1);
  handle->resource = resource;
  handle->list = list;
  handle->window = window;

  wl_resource_set_implementation (resource, &handle_interface, handle,
                                  handle_destroy);

  list->handles = g_list_prepend (list->handles, handle);

  ext_foreign_toplevel_list_v1_send_toplevel (list->resource, resource);

  /* The identifier must be unique per toplevel and never reused; mutter's
   * window id is monotonic and not recycled. */
  identifier = g_strdup_printf ("gnoblin-%" G_GUINT64_FORMAT,
                                meta_window_get_id (window));
  ext_foreign_toplevel_handle_v1_send_identifier (resource, identifier);

  handle_send_title (handle);
  handle_send_app_id (handle);
  ext_foreign_toplevel_handle_v1_send_done (resource);

  handle->unmanaged_id =
    g_signal_connect (window, "unmanaged",
                      G_CALLBACK (on_window_unmanaged), handle);
  handle->notify_title_id =
    g_signal_connect (window, "notify::title",
                      G_CALLBACK (on_window_notify_title), handle);
  handle->notify_wm_class_id =
    g_signal_connect (window, "notify::wm-class",
                      G_CALLBACK (on_window_notify_app_id), handle);
  handle->notify_gtk_app_id_id =
    g_signal_connect (window, "notify::gtk-application-id",
                      G_CALLBACK (on_window_notify_app_id), handle);
}

static void
on_window_created (MetaDisplay *display,
                   MetaWindow  *window,
                   gpointer     user_data)
{
  MetaWaylandForeignToplevelList *list = user_data;

  if (list->stopped)
    return;
  if (!window_is_exposable (window))
    return;

  foreign_toplevel_list_advertise (list, window);
}

static void
foreign_toplevel_list_destroy (struct wl_resource *resource)
{
  MetaWaylandForeignToplevelList *list = wl_resource_get_user_data (resource);
  GList *l;

  g_clear_signal_handler (&list->window_created_id, list->display);

  /* Handles outlive the manager: detach them so their own destructor does not
   * touch freed list state. */
  for (l = list->handles; l; l = l->next)
    {
      MetaWaylandForeignToplevelHandle *handle = l->data;
      handle->list = NULL;
    }
  g_clear_pointer (&list->handles, g_list_free);

  g_free (list);
}

static void
foreign_toplevel_list_handle_stop (struct wl_client   *client,
                                   struct wl_resource *resource)
{
  MetaWaylandForeignToplevelList *list = wl_resource_get_user_data (resource);

  if (list->stopped)
    return;

  list->stopped = TRUE;
  g_clear_signal_handler (&list->window_created_id, list->display);
  ext_foreign_toplevel_list_v1_send_finished (resource);
}

static void
foreign_toplevel_list_handle_destroy (struct wl_client   *client,
                                      struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_foreign_toplevel_list_v1_interface list_interface = {
  foreign_toplevel_list_handle_stop,
  foreign_toplevel_list_handle_destroy,
};

static void
bind_foreign_toplevel_list (struct wl_client *client,
                            void             *data,
                            uint32_t          version,
                            uint32_t          id)
{
  MetaWaylandCompositor *compositor = data;
  MetaContext *context = meta_wayland_compositor_get_context (compositor);
  MetaDisplay *display = meta_context_get_display (context);
  MetaWaylandForeignToplevelList *list;
  struct wl_resource *resource;
  GList *windows, *l;

  resource = wl_resource_create (client,
                                 &ext_foreign_toplevel_list_v1_interface,
                                 version,
                                 id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  list = g_new0 (MetaWaylandForeignToplevelList, 1);
  list->resource = resource;
  list->display = display;

  wl_resource_set_implementation (resource, &list_interface, list,
                                  foreign_toplevel_list_destroy);

  list->window_created_id =
    g_signal_connect (display, "window-created",
                      G_CALLBACK (on_window_created), list);

  windows = meta_display_list_all_windows (display);
  for (l = windows; l; l = l->next)
    {
      MetaWindow *window = l->data;

      if (window_is_exposable (window))
        foreign_toplevel_list_advertise (list, window);
    }
  g_list_free (windows);
}

void
meta_wayland_init_foreign_toplevel_list (MetaWaylandCompositor *compositor)
{
  if (!gnoblin_config_get_bool ("protocols", "ext-foreign-toplevel-list", TRUE))
    {
      g_message ("Gnoblin ext-foreign-toplevel-list protocol disabled by settings");
      return;
    }

  if (!wl_global_create (compositor->wayland_display,
                         &ext_foreign_toplevel_list_v1_interface,
                         META_EXT_FOREIGN_TOPLEVEL_LIST_VERSION,
                         compositor,
                         bind_foreign_toplevel_list))
    g_error ("Failed to register ext-foreign-toplevel-list global");
}
