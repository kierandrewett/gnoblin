/*
 * gnoblin: ext-idle-notify-v1 support for mutter.
 *
 * Implements the ext-idle-notify-v1 protocol on top of mutter's existing
 * MetaIdleMonitor. This lets standard Wayland idle daemons such as swayidle
 * and hypridle drive idle-triggered actions (screen blank, DPMS, lock) inside
 * a Gnoblin session, instead of relying on GNOME's private D-Bus IdleMonitor.
 *
 * Each ext_idle_notification_v1 maps to a persistent idle watch (fires the
 * "idled" event) plus, while idle, a one-shot user-active watch (fires the
 * "resumed" event and re-arms). get_idle_notification respects idle inhibitors;
 * get_input_idle_notification (v2) ignores them.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-idle-notify.h"

#include <gio/gio.h>
#include <wayland-server.h>

#include "meta/meta-backend.h"
#include "meta/meta-context.h"
#include "meta/meta-idle-monitor.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/gnoblin-config.h"

#include "ext-idle-notify-v1-server-protocol.h"

#define META_EXT_IDLE_NOTIFY_VERSION 2

typedef struct _MetaWaylandIdleNotification
{
  struct wl_resource *resource;
  MetaIdleMonitor *monitor;
  guint64 timeout_msec;
  MetaIdleMonitorWatchFlags flags;
  guint idle_watch_id;   /* persistent: fires "idled" */
  guint active_watch_id; /* one-shot, set only while idle: fires "resumed" */
  gboolean is_idle;
} MetaWaylandIdleNotification;

static void arm_idle_watch (MetaWaylandIdleNotification *notification);

static void
on_user_active (MetaIdleMonitor *monitor,
                guint            watch_id,
                gpointer         user_data)
{
  MetaWaylandIdleNotification *notification = user_data;

  /* User-active watches are one-shot: the monitor removes this watch itself
   * after firing, so we must not remove it again. */
  notification->active_watch_id = 0;

  if (!notification->is_idle)
    return;

  notification->is_idle = FALSE;
  ext_idle_notification_v1_send_resumed (notification->resource);
}

static void
on_idle (MetaIdleMonitor *monitor,
         guint            watch_id,
         gpointer         user_data)
{
  MetaWaylandIdleNotification *notification = user_data;

  if (notification->is_idle)
    return;

  notification->is_idle = TRUE;
  ext_idle_notification_v1_send_idled (notification->resource);

  /* Watch for the user becoming active again so we can send "resumed". The
   * persistent idle watch stays armed and will fire again on the next idle
   * period. */
  if (notification->active_watch_id == 0)
    {
      notification->active_watch_id =
        meta_idle_monitor_add_user_active_watch (notification->monitor,
                                                 on_user_active,
                                                 notification,
                                                 NULL);
    }
}

static void
arm_idle_watch (MetaWaylandIdleNotification *notification)
{
  guint64 interval;

  /* A 0 interval is a user-active watch in MetaIdleMonitor; clamp to 1ms so a
   * zero protocol timeout ("notify as soon as inactive") stays an idle watch. */
  interval = notification->timeout_msec > 0 ? notification->timeout_msec : 1;

  notification->idle_watch_id =
    meta_idle_monitor_add_idle_watch_full (notification->monitor,
                                           interval,
                                           on_idle,
                                           notification,
                                           NULL,
                                           notification->flags);
}

static void
idle_notification_destroy (struct wl_resource *resource)
{
  MetaWaylandIdleNotification *notification =
    wl_resource_get_user_data (resource);

  if (notification->idle_watch_id != 0)
    meta_idle_monitor_remove_watch (notification->monitor,
                                    notification->idle_watch_id);
  if (notification->active_watch_id != 0)
    meta_idle_monitor_remove_watch (notification->monitor,
                                    notification->active_watch_id);

  g_free (notification);
}

static void
idle_notification_handle_destroy (struct wl_client   *client,
                                  struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_idle_notification_v1_interface notification_interface = {
  idle_notification_handle_destroy,
};

static void
create_notification (struct wl_client          *client,
                     struct wl_resource        *manager_resource,
                     uint32_t                   id,
                     uint32_t                   timeout,
                     struct wl_resource        *seat_resource,
                     MetaIdleMonitorWatchFlags  flags)
{
  MetaWaylandCompositor *compositor =
    wl_resource_get_user_data (manager_resource);
  MetaContext *context = meta_wayland_compositor_get_context (compositor);
  MetaBackend *backend = meta_context_get_backend (context);
  MetaWaylandIdleNotification *notification;
  struct wl_resource *resource;

  resource = wl_resource_create (client,
                                 &ext_idle_notification_v1_interface,
                                 wl_resource_get_version (manager_resource),
                                 id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  notification = g_new0 (MetaWaylandIdleNotification, 1);
  notification->resource = resource;
  notification->monitor = meta_backend_get_core_idle_monitor (backend);
  notification->timeout_msec = timeout;
  notification->flags = flags;

  wl_resource_set_implementation (resource,
                                  &notification_interface,
                                  notification,
                                  idle_notification_destroy);

  arm_idle_watch (notification);
}

static void
notifier_get_idle_notification (struct wl_client   *client,
                                struct wl_resource *resource,
                                uint32_t            id,
                                uint32_t            timeout,
                                struct wl_resource *seat_resource)
{
  create_notification (client, resource, id, timeout, seat_resource,
                       META_IDLE_MONITOR_WATCH_FLAGS_NONE);
}

static void
notifier_get_input_idle_notification (struct wl_client   *client,
                                      struct wl_resource *resource,
                                      uint32_t            id,
                                      uint32_t            timeout,
                                      struct wl_resource *seat_resource)
{
  create_notification (client, resource, id, timeout, seat_resource,
                       META_IDLE_MONITOR_WATCH_FLAGS_UNINHIBITABLE);
}

static void
notifier_destroy (struct wl_client   *client,
                  struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_idle_notifier_v1_interface notifier_interface = {
  notifier_destroy,
  notifier_get_idle_notification,
  notifier_get_input_idle_notification,
};

static void
bind_idle_notifier (struct wl_client *client,
                    void             *data,
                    uint32_t          version,
                    uint32_t          id)
{
  MetaWaylandCompositor *compositor = data;
  struct wl_resource *resource;

  resource = wl_resource_create (client,
                                 &ext_idle_notifier_v1_interface,
                                 version,
                                 id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  wl_resource_set_implementation (resource,
                                  &notifier_interface,
                                  compositor,
                                  NULL);
}

void
meta_wayland_init_idle_notify (MetaWaylandCompositor *compositor)
{
  if (!gnoblin_config_protocol_enabled ("ext-idle-notify"))
    {
      g_message ("Gnoblin ext-idle-notify protocol disabled by settings");
      return;
    }

  if (!wl_global_create (compositor->wayland_display,
                         &ext_idle_notifier_v1_interface,
                         META_EXT_IDLE_NOTIFY_VERSION,
                         compositor,
                         bind_idle_notifier))
    g_error ("Failed to register ext-idle-notify global");
}
