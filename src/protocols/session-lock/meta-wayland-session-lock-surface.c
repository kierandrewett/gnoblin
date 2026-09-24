/* ext-session-lock-v1 surface role for Gnoblin Mutter. */

#include "config.h"

#include "wayland/meta-wayland-session-lock-surface.h"

#include "backends/meta-backend-private.h"
#include "backends/meta-logical-monitor-private.h"
#include "backends/meta-monitor-private.h"
#include "backends/meta-monitor-manager-private.h"
#include "core/meta-context-private.h"
#include "core/display-private.h"
#include "core/window-private.h"
#include "wayland/meta-wayland-outputs.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-session-lock.h"
#include "wayland/meta-wayland-surface-private.h"
#include "wayland/meta-wayland-window-configuration.h"
#include "wayland/meta-wayland-actor-surface.h"
#include "wayland/meta-window-wayland.h"

#include "ext-session-lock-v1-server-protocol.h"

typedef struct
{
  uint32_t serial;
  int width;
  int height;
} Configure;

struct _MetaWaylandSessionLockSurface
{
  MetaWaylandShellSurface parent;

  struct wl_resource *resource;
  MetaWaylandOutput *output;
  gpointer owner;
  GQueue configures; /* Configure, in wire order */
  int configured_width;
  int configured_height;
  int acked_width;
  int acked_height;
  gboolean has_acked_configure;
  gboolean mapped;
  gboolean closed;
  gulong output_destroyed_id;
};

G_DEFINE_TYPE (MetaWaylandSessionLockSurface,
               meta_wayland_session_lock_surface,
               META_TYPE_WAYLAND_SHELL_SURFACE)

static MtkRectangle
get_output_layout (MetaWaylandSessionLockSurface *lock_surface)
{
  MetaWaylandSurface *surface =
    meta_wayland_surface_role_get_surface (META_WAYLAND_SURFACE_ROLE (lock_surface));
  MetaContext *context = meta_wayland_compositor_get_context (surface->compositor);
  MetaBackend *backend = meta_context_get_backend (context);
  MetaMonitorManager *manager = meta_backend_get_monitor_manager (backend);
  MetaLogicalMonitor *logical_monitor = NULL;
  MetaMonitor *monitor;

  monitor = lock_surface->output ?
    meta_wayland_output_get_monitor (lock_surface->output) : NULL;
  if (monitor)
    logical_monitor = meta_monitor_get_logical_monitor (monitor);
  if (!logical_monitor)
    logical_monitor = meta_monitor_manager_get_primary_logical_monitor (manager);

  return logical_monitor ? meta_logical_monitor_get_layout (logical_monitor) :
    (MtkRectangle) { 0, 0, 1, 1 };
}

static MetaDisplay *
display_from_surface (MetaWaylandSurface *surface)
{
  MetaContext *context = meta_wayland_compositor_get_context (surface->compositor);

  return meta_context_get_display (context);
}

static void
place_actor_on_output (MetaWaylandSessionLockSurface *lock_surface,
                       MetaSurfaceActor              *actor)
{
  MtkRectangle layout = get_output_layout (lock_surface);
  MetaWaylandSurface *surface =
    meta_wayland_session_lock_surface_get_wayland_surface (lock_surface);
  MetaWindow *window = meta_wayland_surface_get_window (surface);

  /* The private lock scene is stage-relative. MetaSurfaceActor coordinates are
   * normally relative to MetaWindowActor, so preserve the assigned output's
   * logical origin while it is parented directly under the lock scene. */
  clutter_actor_set_position (CLUTTER_ACTOR (actor), layout.x, layout.y);
  if (window)
    meta_window_move_resize_frame (window, FALSE, layout.x, layout.y,
                                   layout.width, layout.height);
}

static void
send_configure (MetaWaylandSessionLockSurface *lock_surface)
{
  MetaWaylandSurface *surface =
    meta_wayland_surface_role_get_surface (META_WAYLAND_SURFACE_ROLE (lock_surface));
  MtkRectangle layout = get_output_layout (lock_surface);
  Configure *configure = g_new (Configure, 1);

  configure->serial = wl_display_next_serial (surface->compositor->wayland_display);
  configure->width = MAX (layout.width, 1);
  configure->height = MAX (layout.height, 1);
  lock_surface->configured_width = configure->width;
  lock_surface->configured_height = configure->height;
  g_queue_push_tail (&lock_surface->configures, configure);
  ext_session_lock_surface_v1_send_configure (lock_surface->resource,
                                               configure->serial,
                                               configure->width,
                                               configure->height);
}

void
meta_wayland_session_lock_surface_send_configure (MetaWaylandSessionLockSurface *surface)
{
  if (surface->resource && !surface->closed)
    send_configure (surface);
}

static void
output_destroyed (MetaWaylandOutput                    *output,
                  MetaWaylandSessionLockSurface        *lock_surface)
{
  lock_surface->output_destroyed_id = 0;
  meta_wayland_session_lock_surface_destroyed (lock_surface->owner, lock_surface);
  lock_surface->output = NULL;
  /* The controller's opaque cover remains authoritative.  This object cannot
   * safely migrate to another output after its output has disappeared. */
  meta_wayland_session_lock_surface_close (lock_surface);
}

static void
ack_configure (struct wl_client   *client,
               struct wl_resource *resource,
               uint32_t            serial)
{
  MetaWaylandSessionLockSurface *lock_surface = wl_resource_get_user_data (resource);
  Configure *configure;

  if (lock_surface->closed)
    return;
  configure = g_queue_peek_head (&lock_surface->configures);
  while (configure && configure->serial != serial)
    {
      g_free (g_queue_pop_head (&lock_surface->configures));
      configure = g_queue_peek_head (&lock_surface->configures);
    }
  if (!configure)
    {
      wl_resource_post_error (resource,
                              EXT_SESSION_LOCK_SURFACE_V1_ERROR_INVALID_SERIAL,
                              "unknown session-lock configure serial %u", serial);
      return;
    }

  lock_surface->acked_width = configure->width;
  lock_surface->acked_height = configure->height;
  lock_surface->has_acked_configure = TRUE;
  g_free (g_queue_pop_head (&lock_surface->configures));
}

static void
surface_destroy (struct wl_client *client,
                 struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_session_lock_surface_v1_interface surface_interface = {
  .destroy = surface_destroy,
  .ack_configure = ack_configure,
};

static void
resource_destroyed (struct wl_resource *resource)
{
  MetaWaylandSessionLockSurface *lock_surface = wl_resource_get_user_data (resource);

  if (!lock_surface)
    return;
  wl_resource_set_user_data (resource, NULL);
  lock_surface->resource = NULL;
  meta_wayland_session_lock_surface_destroyed (lock_surface->owner, lock_surface);
  meta_wayland_session_lock_surface_close (lock_surface);
}

static void
apply_state (MetaWaylandSurfaceRole  *surface_role,
             MetaWaylandSurfaceState *pending)
{
  MetaWaylandSessionLockSurface *lock_surface =
    META_WAYLAND_SESSION_LOCK_SURFACE (surface_role);
  MetaWaylandActorSurface *actor_surface = META_WAYLAND_ACTOR_SURFACE (surface_role);
  MetaWaylandSurface *surface = meta_wayland_surface_role_get_surface (surface_role);
  MetaWaylandSurfaceRoleClass *parent_class;

  if (pending->newly_attached && !pending->buffer)
    {
      meta_wayland_actor_surface_queue_frame_callbacks (actor_surface, pending);
      wl_resource_post_error (lock_surface->resource,
                              EXT_SESSION_LOCK_SURFACE_V1_ERROR_NULL_BUFFER,
                              "session-lock surfaces cannot commit a null buffer");
      return;
    }
  if (pending->newly_attached && pending->buffer &&
      !lock_surface->has_acked_configure)
    {
      meta_wayland_actor_surface_queue_frame_callbacks (actor_surface, pending);
      wl_resource_post_error (lock_surface->resource,
                              EXT_SESSION_LOCK_SURFACE_V1_ERROR_COMMIT_BEFORE_FIRST_ACK,
                              "commit before ack_configure");
      return;
    }

  parent_class = META_WAYLAND_SURFACE_ROLE_CLASS (
    meta_wayland_session_lock_surface_parent_class);
  parent_class->apply_state (surface_role, pending);
  if (pending->newly_attached && pending->buffer &&
      (meta_wayland_surface_get_width (surface) != lock_surface->acked_width ||
      meta_wayland_surface_get_height (surface) != lock_surface->acked_height)
      )
    {
      wl_resource_post_error (lock_surface->resource,
                              EXT_SESSION_LOCK_SURFACE_V1_ERROR_DIMENSIONS_MISMATCH,
                              "buffer must match acknowledged configure size");
      return;
    }
  lock_surface->mapped = TRUE;
}

static void
post_apply_state (MetaWaylandSurfaceRole  *surface_role,
                  MetaWaylandSurfaceState *pending)
{
  MetaWaylandSessionLockSurface *lock_surface =
    META_WAYLAND_SESSION_LOCK_SURFACE (surface_role);
  MetaWaylandSurface *surface = meta_wayland_surface_role_get_surface (surface_role);
  MetaSurfaceActor *actor;
  ClutterActor *scene;
  MetaWaylandSurfaceRoleClass *parent_class;

  parent_class = META_WAYLAND_SURFACE_ROLE_CLASS (
    meta_wayland_session_lock_surface_parent_class);
  if (parent_class->post_apply_state)
    parent_class->post_apply_state (surface_role, pending);
  if (!lock_surface->mapped)
    return;

  actor = meta_wayland_actor_surface_get_actor (META_WAYLAND_ACTOR_SURFACE (lock_surface));
  scene = meta_wayland_session_lock_get_scene (surface->compositor);
  if (!actor || !scene)
    return;

  if (clutter_actor_get_parent (CLUTTER_ACTOR (actor)) != scene)
    {
      ClutterActor *parent = clutter_actor_get_parent (CLUTTER_ACTOR (actor));

      /* Clutter deliberately rejects add_child() for a parented actor. Hold a
       * reference across remove_child(): removing the last child can otherwise
       * release the surface actor before it is added to the lock scene. */
      g_object_ref (actor);
      if (parent)
        clutter_actor_remove_child (parent, CLUTTER_ACTOR (actor));
      clutter_actor_add_child (scene, CLUTTER_ACTOR (actor));
      g_object_unref (actor);
    }
  place_actor_on_output (lock_surface, actor);
}

static MetaWaylandSurface *
get_toplevel (MetaWaylandSurfaceRole *surface_role)
{
  return meta_wayland_surface_role_get_surface (surface_role);
}

static void
configure (MetaWaylandShellSurface        *shell_surface,
           MetaWaylandWindowConfiguration *configuration)
{
  MetaWaylandSessionLockSurface *lock_surface =
    META_WAYLAND_SESSION_LOCK_SURFACE (shell_surface);
  MetaSurfaceActor *actor = meta_wayland_actor_surface_get_actor (
    META_WAYLAND_ACTOR_SURFACE (lock_surface));

  if (actor && lock_surface->mapped)
    place_actor_on_output (lock_surface, actor);
  meta_wayland_session_lock_surface_send_configure (
    lock_surface);
}

static void
close_shell_surface (MetaWaylandShellSurface *shell_surface)
{
  meta_wayland_session_lock_surface_close (
    META_WAYLAND_SESSION_LOCK_SURFACE (shell_surface));
}

static void
finalize (GObject *object)
{
  MetaWaylandSessionLockSurface *lock_surface =
    META_WAYLAND_SESSION_LOCK_SURFACE (object);

  if (lock_surface->output && lock_surface->output_destroyed_id)
    g_clear_signal_handler (&lock_surface->output_destroyed_id, lock_surface->output);
  g_clear_pointer (&lock_surface->resource, wl_resource_destroy);
  g_queue_clear_full (&lock_surface->configures, g_free);
  G_OBJECT_CLASS (meta_wayland_session_lock_surface_parent_class)->finalize (object);
}

static void
meta_wayland_session_lock_surface_class_init (MetaWaylandSessionLockSurfaceClass *klass)
{
  GObjectClass *object_class = G_OBJECT_CLASS (klass);
  MetaWaylandSurfaceRoleClass *role_class = META_WAYLAND_SURFACE_ROLE_CLASS (klass);
  MetaWaylandShellSurfaceClass *shell_class = META_WAYLAND_SHELL_SURFACE_CLASS (klass);

  object_class->finalize = finalize;
  role_class->apply_state = apply_state;
  role_class->post_apply_state = post_apply_state;
  role_class->get_toplevel = get_toplevel;
  shell_class->configure = configure;
  shell_class->close = close_shell_surface;
}

static void
meta_wayland_session_lock_surface_init (MetaWaylandSessionLockSurface *lock_surface)
{
  g_queue_init (&lock_surface->configures);
}

MetaWaylandSessionLockSurface *
meta_wayland_session_lock_surface_new (MetaWaylandSurface *surface,
                                       struct wl_client   *client,
                                       uint32_t            id,
                                       struct wl_resource *output_resource,
                                       gpointer             owner)
{
  MetaWaylandSessionLockSurface *lock_surface;

  if (!meta_wayland_surface_assign_role (surface,
                                         META_TYPE_WAYLAND_SESSION_LOCK_SURFACE,
                                         NULL))
    return NULL;
  lock_surface = META_WAYLAND_SESSION_LOCK_SURFACE (surface->role);
  lock_surface->owner = owner;
  lock_surface->output = wl_resource_get_user_data (output_resource);
  lock_surface->resource = wl_resource_create (client,
                                                &ext_session_lock_surface_v1_interface,
                                                1, id);
  wl_resource_set_implementation (lock_surface->resource, &surface_interface,
                                  lock_surface, resource_destroyed);
  lock_surface->output_destroyed_id = g_signal_connect (lock_surface->output,
                                                         "output-destroyed",
                                                         G_CALLBACK (output_destroyed),
                                                         lock_surface);
  {
    MetaWindow *window = meta_window_wayland_new (display_from_surface (surface), surface);

    window->type = META_WINDOW_DOCK;
    window->input = TRUE;
    meta_wayland_shell_surface_set_window (META_WAYLAND_SHELL_SURFACE (lock_surface),
                                           window);
  }
  send_configure (lock_surface);
  return lock_surface;
}

struct wl_resource *
meta_wayland_session_lock_surface_get_resource (MetaWaylandSessionLockSurface *surface)
{
  return surface->resource;
}

MetaWaylandOutput *
meta_wayland_session_lock_surface_get_output (MetaWaylandSessionLockSurface *surface)
{
  return surface->output;
}

MetaWaylandSurface *
meta_wayland_session_lock_surface_get_wayland_surface (MetaWaylandSessionLockSurface *surface)
{
  return meta_wayland_surface_role_get_surface (META_WAYLAND_SURFACE_ROLE (surface));
}

gboolean
meta_wayland_session_lock_surface_is_mapped (MetaWaylandSessionLockSurface *surface)
{
  MetaWaylandSurface *wayland_surface;
  MetaSurfaceActor *actor;

  if (!surface->mapped || surface->closed || !surface->resource)
    return FALSE;
  wayland_surface = meta_wayland_session_lock_surface_get_wayland_surface (surface);
  actor = meta_wayland_surface_get_actor (wayland_surface);
  return actor && clutter_actor_is_mapped (CLUTTER_ACTOR (actor)) &&
         clutter_actor_is_visible (CLUTTER_ACTOR (actor));
}

void
meta_wayland_session_lock_surface_close (MetaWaylandSessionLockSurface *surface)
{
  MetaWaylandSurface *wayland_surface;
  MetaSurfaceActor *actor;
  ClutterActor *scene;

  if (surface->closed)
    return;
  surface->closed = TRUE;
  wayland_surface = meta_wayland_session_lock_surface_get_wayland_surface (surface);
  actor = wayland_surface ? meta_wayland_surface_get_actor (wayland_surface) : NULL;
  scene = wayland_surface ? meta_wayland_session_lock_get_scene (wayland_surface->compositor) : NULL;
  if (actor && scene && clutter_actor_get_parent (CLUTTER_ACTOR (actor)) == scene)
    clutter_actor_remove_child (scene, CLUTTER_ACTOR (actor));
  meta_wayland_shell_surface_destroy_window (META_WAYLAND_SHELL_SURFACE (surface));
}
