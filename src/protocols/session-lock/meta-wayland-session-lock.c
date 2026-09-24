/*
 * Gnoblin session-lock protocol boundary.
 *
 * ext-session-lock-v1 requires the compositor to stop rendering and routing
 * input to normal clients before it emits `locked`.  Mutter has no native
 * session-lock controller: a layer-shell role only controls a window's layer
 * and keyboard focus, so it cannot meet that requirement for pointer, touch,
 * shortcuts, output hotplug, or client death.
 *
 * Keep the global unadvertised until MetaSessionLockController owns all of
 * those paths.  In particular, do not turn this into a cosmetic lock surface:
 * a global is an interoperability promise that a client may safely suspend
 * after receiving `locked`.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "config.h"

#include "backends/meta-backend-private.h"
#include "compositor/meta-surface-actor-wayland.h"
#include "meta/compositor.h"
#include "meta/meta-cursor-tracker.h"
#include "backends/meta-monitor-manager-private.h"
#include "clutter/clutter.h"
#include "core/meta-context-private.h"
#include "core/display-private.h"
#include "wayland/meta-wayland-data-device-primary.h"
#include "wayland/meta-wayland-data-device.h"
#include "wayland/meta-wayland-input.h"
#include "wayland/meta-wayland-keyboard.h"
#include "wayland/meta-wayland-pointer.h"
#include "wayland/meta-wayland-outputs.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-seat.h"
#include "wayland/meta-wayland-session-lock.h"
#include "wayland/meta-wayland-session-lock-surface.h"
#include "wayland/meta-wayland-surface-private.h"
#include "wayland/meta-wayland-tablet-seat.h"
#include "wayland/meta-wayland-text-input.h"
#include "wayland/meta-wayland-touch.h"

#include "wayland/gnoblin-config.h"

#include "ext-session-lock-v1-server-protocol.h"

typedef struct
{
  MetaWaylandCompositor *compositor;
  struct wl_resource *resource;
  struct wl_client *client;
  struct wl_listener client_destroyed;
  gboolean client_destroy_listener_installed;
  gboolean locked_sent;
} MetaWaylandSessionLock;

typedef struct
{
  gint ref_count;
  MetaWaylandSessionLockStateChangedFunc callback;
  gpointer user_data;
  GDestroyNotify destroy_notify;
} MetaWaylandSessionLockStateChangedCallback;

/* This data stays on the compositor object, whose lifetime owns its stage and
 * seat.  It is deliberately separate from the ext-session-lock resource: a
 * client may disappear, whereas the cover and input embargo must survive. */
typedef struct
{
  MetaWaylandSessionLockState state;
  MetaWaylandCompositor *compositor;
  ClutterStage *stage;
  ClutterActor *scene;
  ClutterActor *cover;
  MetaWaylandEventHandler *input_handler;
  /* This is independent from state: a client can die while COVERING, which
   * must enter FAILSAFE for access control but has not yet proven a frame. */
  gboolean presentation_confirmed;
  /* MetaWaylandStageView* -> minimum global frame counter that is allowed to
   * prove this particular cover generation.  A mere view pointer is not
   * enough: a presentation notification queued before the cover/restack can
   * otherwise satisfy the new barrier. */
  GHashTable *unpresented_stage_views;
  gulong stage_presented_id;
  gulong stage_views_changed_id;
  gulong stage_child_added_id;
  gulong stage_before_paint_id;
  MetaMonitorManager *monitor_manager;
  gulong monitors_changed_id;
  MetaCompositor *scene_compositor;
  MetaCursorTracker *cursor_tracker;
  gboolean unredirect_inhibited;
  gboolean cursor_visibility_inhibited;
  MetaWaylandSessionLock *lock;
  GHashTable *surfaces; /* MetaWaylandOutput* -> MetaWaylandSessionLockSurface* */
  GHashTable *state_changed_callbacks; /* gulong -> callback */
  gulong next_state_changed_callback_id;
} MetaWaylandSessionLockController;

#define SESSION_LOCK_CONTROLLER_KEY "gnoblin-session-lock-controller"

static MetaWaylandSessionLockController *
get_controller (MetaWaylandCompositor *compositor);
static void reset_presentation_barrier (MetaWaylandSessionLockController *controller);

static void session_lock_destroy (struct wl_resource *resource);

/* Lock-surface resources outlive the lock object in the wire protocol. Their
 * actors must not outlive the controller ownership that admitted them: a
 * stale role could otherwise remain visible after abort/takeover or be found
 * by a later input route. */
static void
close_lock_surfaces (MetaWaylandSessionLockController *controller)
{
  g_autoptr (GPtrArray) surfaces = NULL;
  GHashTableIter iter;
  gpointer surface;

  surfaces = g_ptr_array_new_with_free_func (g_object_unref);
  g_hash_table_iter_init (&iter, controller->surfaces);
  while (g_hash_table_iter_next (&iter, NULL, &surface))
    g_ptr_array_add (surfaces, g_object_ref (surface));

  /* Closing a surface may synchronously invoke surface_destroyed(), which
   * removes its output mapping. Drop all mappings first so that callback does
   * not mutate an iterator and no stale role can be selected meanwhile. */
  g_hash_table_remove_all (controller->surfaces);
  for (guint i = 0; i < surfaces->len; i++)
    meta_wayland_session_lock_surface_close (
      g_ptr_array_index (surfaces, i));
}

static void
destroy_state_changed_callback (gpointer data)
{
  MetaWaylandSessionLockStateChangedCallback *callback = data;

  if (!g_atomic_int_dec_and_test (&callback->ref_count))
    return;
  if (callback->destroy_notify)
    callback->destroy_notify (callback->user_data);
  g_free (callback);
}

static void
set_state (MetaWaylandSessionLockController *controller,
           MetaWaylandSessionLockState       state)
{
  g_autoptr (GPtrArray) callbacks = NULL;
  MetaWaylandSessionLockStateChangedCallback *callback;
  GHashTableIter iter;
  gpointer value;

  if (controller->state == state)
    return;

  controller->state = state;
  callbacks = g_ptr_array_new_with_free_func (destroy_state_changed_callback);
  g_hash_table_iter_init (&iter, controller->state_changed_callbacks);
  while (g_hash_table_iter_next (&iter, NULL, &value))
    {
      callback = value;
      g_atomic_int_inc (&callback->ref_count);
      g_ptr_array_add (callbacks, callback);
    }

  for (guint i = 0; i < callbacks->len; i++)
    {
      callback = g_ptr_array_index (callbacks, i);
      callback->callback (controller->compositor, state, callback->user_data);
    }
}

static void
session_lock_client_destroyed (struct wl_listener *listener,
                              void               *data)
{
  MetaWaylandSessionLock *lock =
    wl_container_of (listener, lock, client_destroyed);

  /* The controller-owned blackout and grab deliberately outlive the Wayland
   * client. A later session-lock client may take over from FAILSAFE. */
  meta_wayland_session_lock_enter_failsafe (lock->compositor);
  close_lock_surfaces (get_controller (lock->compositor));
  set_state (get_controller (lock->compositor), META_WAYLAND_SESSION_LOCK_FAILSAFE);
  lock->client = NULL;
}

static void
session_lock_unlock_and_destroy (struct wl_client   *client,
                                 struct wl_resource *resource)
{
  MetaWaylandSessionLock *lock = wl_resource_get_user_data (resource);
  MetaWaylandSessionLockController *controller;

  if (!lock->locked_sent)
    {
      wl_resource_post_error (resource,
                              EXT_SESSION_LOCK_V1_ERROR_INVALID_UNLOCK,
                              "unlock requested before the compositor confirmed lock");
      return;
    }

  controller = get_controller (lock->compositor);
  if (controller->lock != lock || lock->client != client)
    {
      wl_resource_post_error (resource, EXT_SESSION_LOCK_V1_ERROR_INVALID_UNLOCK,
                              "only the active lock owner can unlock");
      return;
    }

  /* Restore in one compositor-main-loop transaction. The event embargo is
   * removed only after the opaque scene has gone, and observers see UNLOCKED
   * only after both cursor and direct-scanout holds are balanced. */
  close_lock_surfaces (controller);
  if (controller->scene)
    {
      clutter_actor_destroy (controller->scene);
      controller->scene = NULL;
      controller->cover = NULL;
    }
  if (controller->input_handler && controller->compositor->seat)
    {
      meta_wayland_input_detach_event_handler (
        controller->compositor->seat->input_handler, controller->input_handler);
      controller->input_handler = NULL;
    }
  if (controller->cursor_visibility_inhibited)
    {
      meta_cursor_tracker_uninhibit_cursor_visibility (controller->cursor_tracker);
      controller->cursor_visibility_inhibited = FALSE;
    }
  if (controller->unredirect_inhibited)
    {
      meta_compositor_enable_unredirect (controller->scene_compositor);
      controller->unredirect_inhibited = FALSE;
    }
  controller->lock = NULL;
  controller->presentation_confirmed = FALSE;
  set_state (controller, META_WAYLAND_SESSION_LOCK_UNLOCKED);
  wl_resource_destroy (resource);
}

static void
session_lock_destroy_request (struct wl_client   *client,
                              struct wl_resource *resource)
{
  MetaWaylandSessionLock *lock = wl_resource_get_user_data (resource);

  if (lock->locked_sent)
    {
      wl_resource_post_error (resource,
                              EXT_SESSION_LOCK_V1_ERROR_INVALID_DESTROY,
                              "a locked session must use unlock_and_destroy");
      return;
    }
  wl_resource_destroy (resource);
}

static void
session_lock_get_lock_surface (struct wl_client   *client,
                               struct wl_resource *resource,
                               uint32_t            id,
                               struct wl_resource *surface,
                               struct wl_resource *output)
{
  MetaWaylandSessionLock *lock = wl_resource_get_user_data (resource);
  MetaWaylandSessionLockController *controller;
  MetaWaylandSurface *wayland_surface;
  MetaWaylandOutput *wayland_output;
  MetaWaylandSurfaceState *pending;
  MetaWaylandSessionLockSurface *lock_surface;

  if (!lock || lock->client != client)
    return;
  controller = get_controller (lock->compositor);
  wayland_surface = wl_resource_get_user_data (surface);
  wayland_output = wl_resource_get_user_data (output);
  if (!wayland_surface || !wayland_output)
    {
      wl_resource_post_error (resource, EXT_SESSION_LOCK_V1_ERROR_ROLE,
                              "lock surface requires a live wl_surface and wl_output");
      return;
    }
  if (g_hash_table_contains (controller->surfaces, wayland_output))
    {
      wl_resource_post_error (resource, EXT_SESSION_LOCK_V1_ERROR_DUPLICATE_OUTPUT,
                              "one lock surface is allowed per output");
      return;
    }
  pending = meta_wayland_surface_get_pending_state (wayland_surface);
  if ((pending->newly_attached && pending->buffer) ||
      meta_wayland_surface_get_buffer (wayland_surface) ||
      meta_wayland_surface_has_initial_commit (wayland_surface))
    {
      wl_resource_post_error (resource, EXT_SESSION_LOCK_V1_ERROR_ROLE,
                              "lock surface must be assigned before its first buffer or commit");
      return;
    }
  lock_surface = meta_wayland_session_lock_surface_new (wayland_surface, client,
                                                         id, output, controller);
  if (!lock_surface)
    {
      wl_resource_post_error (resource, EXT_SESSION_LOCK_V1_ERROR_ROLE,
                              "wl_surface already has a role");
      return;
    }
  g_hash_table_insert (controller->surfaces, wayland_output, lock_surface);
}

static const struct ext_session_lock_v1_interface session_lock_interface = {
  .destroy = session_lock_destroy_request,
  .get_lock_surface = session_lock_get_lock_surface,
  .unlock_and_destroy = session_lock_unlock_and_destroy,
};

static void
session_lock_destroy (struct wl_resource *resource)
{
  MetaWaylandSessionLock *lock = wl_resource_get_user_data (resource);
  MetaWaylandSessionLockController *controller;

  if (!lock)
    return;
  wl_resource_set_user_data (resource, NULL);
  if (lock->client_destroy_listener_installed)
    wl_list_remove (&lock->client_destroyed.link);
  controller = get_controller (lock->compositor);
  if (controller->lock == lock)
    {
      /* `destroy` is legal only before `locked`, but it may arrive after the
       * cover transition has begun.  Never turn that client-side abort into
       * an unlock: retain the compositor-owned cover and let a later lock
       * object take over through the FAILSAFE path. */
      controller->lock = NULL;
      close_lock_surfaces (controller);
      meta_wayland_session_lock_enter_failsafe (lock->compositor);
      set_state (controller, META_WAYLAND_SESSION_LOCK_FAILSAFE);
    }
  g_free (lock);
}

static void
finished_lock_destroy (struct wl_client   *client,
                       struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static void
finished_lock_get_lock_surface (struct wl_client   *client,
                                struct wl_resource *resource,
                                uint32_t            id,
                                struct wl_resource *surface,
                                struct wl_resource *output)
{
  wl_resource_post_error (resource, EXT_SESSION_LOCK_V1_ERROR_ROLE,
                          "a finished session lock cannot create surfaces");
}

static void
finished_lock_unlock_and_destroy (struct wl_client   *client,
                                  struct wl_resource *resource)
{
  wl_resource_post_error (resource, EXT_SESSION_LOCK_V1_ERROR_INVALID_UNLOCK,
                          "a finished session lock cannot unlock");
}

static const struct ext_session_lock_v1_interface finished_lock_interface = {
  .destroy = finished_lock_destroy,
  .get_lock_surface = finished_lock_get_lock_surface,
  .unlock_and_destroy = finished_lock_unlock_and_destroy,
};

static void
session_lock_manager_lock (struct wl_client   *client,
                           struct wl_resource *resource,
                           uint32_t            id)
{
  MetaWaylandCompositor *compositor = wl_resource_get_user_data (resource);
  MetaWaylandSessionLockController *controller = get_controller (compositor);
  MetaWaylandSessionLock *lock;

  if (controller->lock)
    {
      struct wl_resource *finished =
        wl_resource_create (client, &ext_session_lock_v1_interface, 1, id);

      if (!finished)
        {
          wl_client_post_no_memory (client);
          return;
        }
      wl_resource_set_implementation (finished, &finished_lock_interface,
                                      NULL, NULL);
      ext_session_lock_v1_send_finished (finished);
      return;
    }

  lock = g_new0 (MetaWaylandSessionLock, 1);
  lock->compositor = compositor;
  lock->client = client;
  lock->resource = wl_resource_create (client, &ext_session_lock_v1_interface, 1, id);
  if (!lock->resource)
    {
      g_free (lock);
      wl_client_post_no_memory (client);
      return;
    }
  wl_resource_set_implementation (lock->resource, &session_lock_interface,
                                  lock, session_lock_destroy);
  controller->lock = lock;
  lock->client_destroyed.notify = session_lock_client_destroyed;
  wl_client_add_destroy_listener (client, &lock->client_destroyed);
  lock->client_destroy_listener_installed = TRUE;
  if (controller->state == META_WAYLAND_SESSION_LOCK_FAILSAFE)
    {
      /* A replacement owner takes responsibility while the original opaque
       * scene and input embargo remain intact. Require a fresh presented
       * covered frame before it receives `locked`. */
      close_lock_surfaces (controller);
      reset_presentation_barrier (controller);
    }
  else
    meta_wayland_session_lock_enter_failsafe (compositor);
}

static void
session_lock_manager_destroy (struct wl_client *client, struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_session_lock_manager_v1_interface session_lock_manager_interface G_GNUC_UNUSED = {
  .destroy = session_lock_manager_destroy,
  .lock = session_lock_manager_lock,
};

static gboolean
consume_event (MetaWaylandEventHandler *handler,
               const ClutterEvent      *event,
               gpointer                 user_data)
{
  /* While no verified lock-surface owner is installed, every event including
   * keyboard shortcuts, pointer, touch, tablet and input-method traffic stops
   * here.  A future owner route must replace this handler atomically; it must
   * never chain normal-session input while the cover is present. */
  return CLUTTER_EVENT_STOP;
}

static MetaWaylandSurface *
get_no_focus_surface (MetaWaylandEventHandler *handler,
                      ClutterFocus            *focus,
                      gpointer                 user_data)
{
  return NULL;
}

static void
clear_focus (MetaWaylandEventHandler *handler,
             ClutterFocus            *focus,
             MetaWaylandSurface      *surface,
             gpointer                 user_data)
{
  MetaWaylandCompositor *compositor = user_data;
  MetaWaylandSeat *seat = compositor->seat;

  meta_wayland_seat_set_input_focus (seat, NULL);
  meta_wayland_data_device_end_drag (&seat->data_device);
  meta_wayland_keyboard_set_focus (seat->keyboard, NULL);
  meta_wayland_pointer_focus_surface (seat->pointer, NULL);
  meta_wayland_touch_cancel (seat->touch);
  meta_wayland_text_input_set_focus (seat->text_input, NULL);
  meta_wayland_tablet_seat_set_pad_focus (seat->tablet_seat, NULL);
  meta_wayland_data_device_set_focus (&seat->data_device, NULL);
  meta_wayland_data_device_primary_set_focus (&seat->primary_data_device,
                                              NULL);
}

static const MetaWaylandEventInterface failsafe_input_interface = {
  .get_focus_surface = get_no_focus_surface,
  .focus = clear_focus,
  .motion = consume_event,
  .press = consume_event,
  .release = consume_event,
  .key = consume_event,
  .other = consume_event,
};

static void
reset_presentation_barrier (MetaWaylandSessionLockController *controller)
{
  GList *l;
  int64_t minimum_fresh_frame;

  g_hash_table_remove_all (controller->unpresented_stage_views);
  controller->presentation_confirmed = FALSE;
  /* Present notifications can be delivered after a transition that queued a
   * cover redraw.  Require a strictly newer stage frame, so only a frame
   * submitted after this barrier may establish locked presentation. */
  minimum_fresh_frame = clutter_stage_get_frame_counter (controller->stage);
  for (l = clutter_stage_peek_stage_views (controller->stage); l; l = l->next)
    {
      int64_t *minimum_frame = g_new (int64_t, 1);

      *minimum_frame = minimum_fresh_frame;
      g_hash_table_insert (controller->unpresented_stage_views, l->data,
                           minimum_frame);
    }

  /* A stage with no views cannot prove that a locked frame reached an output.
   * Keep the controller in COVERING until a view appears and presents. */
  set_state (controller, META_WAYLAND_SESSION_LOCK_COVERING);
  clutter_actor_queue_redraw (CLUTTER_ACTOR (controller->stage));
}

static void
on_stage_views_changed (ClutterActor                     *stage,
                        MetaWaylandSessionLockController *controller)
{
  if (controller->state != META_WAYLAND_SESSION_LOCK_UNLOCKED)
    reset_presentation_barrier (controller);
}

static void
on_stage_child_added (ClutterActor                     *stage,
                      ClutterActor                     *child,
                      MetaWaylandSessionLockController *controller)
{
  if (controller->scene && child != controller->scene)
    {
      clutter_actor_set_child_above_sibling (stage, controller->scene, NULL);
      /* The newly-added child briefly changed the stack.  Do not retain a
       * presentation proof from before the cover was restored to the top. */
      reset_presentation_barrier (controller);
    }
}

static void
on_stage_before_paint (ClutterStage                     *stage,
                       ClutterStageView                 *stage_view,
                       ClutterFrame                     *frame,
                       MetaWaylandSessionLockController *controller)
{
  if (!controller->scene || controller->state == META_WAYLAND_SESSION_LOCK_UNLOCKED)
    return;
  if (clutter_actor_get_last_child (CLUTTER_ACTOR (stage)) != controller->scene)
    {
      clutter_actor_set_child_above_sibling (CLUTTER_ACTOR (stage),
                                              controller->scene, NULL);
      /* A frame queued before this restack cannot prove the cover was last. */
      reset_presentation_barrier (controller);
    }
}

static void
on_monitors_changed (MetaMonitorManager                 *manager,
                     MetaWaylandSessionLockController *controller)
{
  GHashTableIter iter;
  gpointer surface;

  if (controller->state == META_WAYLAND_SESSION_LOCK_UNLOCKED)
    return;
  reset_presentation_barrier (controller);
  g_hash_table_iter_init (&iter, controller->surfaces);
  while (g_hash_table_iter_next (&iter, NULL, &surface))
    meta_wayland_session_lock_surface_send_configure (surface);
}

static void
on_stage_presented (ClutterStage                         *stage,
                    ClutterStageView                     *stage_view,
                    ClutterFrameInfo                     *frame_info,
                    MetaWaylandSessionLockController *controller)
{
  int64_t *minimum_frame;

  if ((controller->state != META_WAYLAND_SESSION_LOCK_COVERING &&
       controller->state != META_WAYLAND_SESSION_LOCK_FAILSAFE) ||
      !clutter_actor_is_effectively_on_stage_view (controller->cover,
                                                    stage_view))
    return;

  minimum_frame = g_hash_table_lookup (controller->unpresented_stage_views,
                                       stage_view);
  if (!minimum_frame || frame_info->global_frame_counter <= *minimum_frame)
    return;

  g_hash_table_remove (controller->unpresented_stage_views, stage_view);
  if (g_hash_table_size (controller->unpresented_stage_views) == 0)
    {
      controller->presentation_confirmed = TRUE;
      if (controller->lock && !controller->lock->locked_sent)
        {
          /* The opaque compositor cover is the presentation proof.  Lock
           * surfaces may refine it afterwards, but can never delay safety. */
          controller->lock->locked_sent = TRUE;
          ext_session_lock_v1_send_locked (controller->lock->resource);
          set_state (controller, META_WAYLAND_SESSION_LOCK_LOCKED);
        }
      else if (controller->lock)
        {
          /* Output hotplug/reset starts a new presentation generation. The
           * owner was already notified, so restore the stable LOCKED state
           * only after every current view has presented the fresh cover. */
          set_state (controller, META_WAYLAND_SESSION_LOCK_LOCKED);
        }
      else if (!controller->lock)
        set_state (controller, META_WAYLAND_SESSION_LOCK_FAILSAFE);
    }
}

static void
destroy_controller (gpointer data)
{
  MetaWaylandSessionLockController *controller = data;

  g_clear_signal_handler (&controller->stage_presented_id, controller->stage);
  g_clear_signal_handler (&controller->stage_views_changed_id, controller->stage);
  g_clear_signal_handler (&controller->stage_child_added_id, controller->stage);
  g_clear_signal_handler (&controller->stage_before_paint_id, controller->stage);
  g_clear_signal_handler (&controller->monitors_changed_id, controller->monitor_manager);

  if (controller->input_handler && controller->compositor->seat)
    meta_wayland_input_detach_event_handler (
      controller->compositor->seat->input_handler, controller->input_handler);

  if (controller->cursor_visibility_inhibited)
    meta_cursor_tracker_uninhibit_cursor_visibility (controller->cursor_tracker);
  if (controller->unredirect_inhibited)
    meta_compositor_enable_unredirect (controller->scene_compositor);

  if (controller->scene)
    clutter_actor_destroy (controller->scene);

  g_clear_pointer (&controller->unpresented_stage_views, g_hash_table_unref);
  g_clear_pointer (&controller->surfaces, g_hash_table_unref);
  g_clear_pointer (&controller->state_changed_callbacks, g_hash_table_unref);
  g_free (controller);
}

static MetaWaylandSessionLockController *
get_controller (MetaWaylandCompositor *compositor)
{
  MetaWaylandSessionLockController *controller;

  controller = g_object_get_data (G_OBJECT (compositor),
                                  SESSION_LOCK_CONTROLLER_KEY);
  if (controller)
    return controller;

  controller = g_new0 (MetaWaylandSessionLockController, 1);
  controller->state = META_WAYLAND_SESSION_LOCK_UNLOCKED;
  controller->compositor = compositor;
  controller->unpresented_stage_views = g_hash_table_new_full (g_direct_hash,
                                                                g_direct_equal,
                                                                NULL, g_free);
  controller->surfaces = g_hash_table_new (g_direct_hash, g_direct_equal);
  controller->state_changed_callbacks =
    g_hash_table_new_full (g_direct_hash, g_direct_equal, NULL,
                           destroy_state_changed_callback);
  g_object_set_data_full (G_OBJECT (compositor),
                          SESSION_LOCK_CONTROLLER_KEY,
                          controller, destroy_controller);
  return controller;
}

void
meta_wayland_session_lock_enter_failsafe (MetaWaylandCompositor *compositor)
{
  MetaWaylandSessionLockController *controller;
  MetaContext *context;
  MetaBackend *backend;
  ClutterActor *stage;

  g_return_if_fail (compositor != NULL);

  controller = get_controller (compositor);
  if (controller->state != META_WAYLAND_SESSION_LOCK_UNLOCKED)
    return;

  context = meta_wayland_compositor_get_context (compositor);
  backend = meta_context_get_backend (context);
  stage = meta_backend_get_stage (backend);
  controller->stage = CLUTTER_STAGE (stage);
  controller->monitor_manager = meta_backend_get_monitor_manager (backend);
  controller->scene_compositor =
    meta_display_get_compositor (meta_context_get_display (context));
  controller->cursor_tracker = meta_backend_get_cursor_tracker (backend);
  meta_compositor_disable_unredirect (controller->scene_compositor);
  controller->unredirect_inhibited = TRUE;
  meta_cursor_tracker_inhibit_cursor_visibility (controller->cursor_tracker);
  controller->cursor_visibility_inhibited = TRUE;

  /* This private scene is the future parent of lock-surface actors.  Its opaque
   * base always stays below those actors and above normal scene content.  The
   * bind constraint follows changed stage allocation, so hotplug and output
   * resize cannot expose an uncovered rectangle while the input embargo is in
   * force. */
  controller->scene = clutter_actor_new ();
  clutter_actor_add_constraint (controller->scene,
                                clutter_bind_constraint_new (stage,
                                                             CLUTTER_BIND_ALL,
                                                             0));
  clutter_actor_set_reactive (controller->scene, FALSE);
  clutter_actor_add_child (stage, controller->scene);

  controller->cover = clutter_actor_new ();
  clutter_actor_set_background_color (controller->cover,
                                      &COGL_COLOR_INIT (0, 0, 0, 255));
  clutter_actor_add_constraint (controller->cover,
                                clutter_bind_constraint_new (controller->scene,
                                                             CLUTTER_BIND_ALL,
                                                             0));
  clutter_actor_set_reactive (controller->cover, FALSE);
  clutter_actor_add_child (controller->scene, controller->cover);

  controller->stage_presented_id =
    g_signal_connect (stage, "presented", G_CALLBACK (on_stage_presented),
                      controller);
  controller->stage_views_changed_id =
    g_signal_connect (stage, "stage-views-changed",
                      G_CALLBACK (on_stage_views_changed), controller);
  controller->stage_child_added_id =
    g_signal_connect (stage, "child-added", G_CALLBACK (on_stage_child_added), controller);
  controller->stage_before_paint_id =
    g_signal_connect (stage, "before-paint", G_CALLBACK (on_stage_before_paint), controller);
  controller->monitors_changed_id =
    g_signal_connect (controller->monitor_manager, "monitors-changed-internal",
                      G_CALLBACK (on_monitors_changed), controller);
  reset_presentation_barrier (controller);

  controller->input_handler =
    meta_wayland_input_attach_event_handler (compositor->seat->input_handler,
                                              &failsafe_input_interface,
                                              TRUE,
                                              compositor);
}

MetaWaylandSessionLockState
meta_wayland_session_lock_get_state (MetaWaylandCompositor *compositor)
{
  g_return_val_if_fail (compositor != NULL,
                        META_WAYLAND_SESSION_LOCK_FAILSAFE);

  return get_controller (compositor)->state;
}

guint
meta_wayland_session_lock_get_capability (MetaWaylandCompositor *compositor)
{
  return 0;
}

gboolean
meta_wayland_session_lock_is_active (MetaWaylandCompositor *compositor)
{
  return meta_wayland_session_lock_get_state (compositor) !=
         META_WAYLAND_SESSION_LOCK_UNLOCKED;
}

gboolean
meta_wayland_session_lock_filter_event (MetaWaylandCompositor *compositor,
                                        const ClutterEvent     *event)
{
  MetaWaylandSessionLockController *controller;
  MetaWaylandSessionLockSurface *lock_surface = NULL;
  MetaWaylandSurface *surface = NULL;
  ClutterEventType event_type;
  GHashTableIter iter;
  gpointer value;

  g_return_val_if_fail (compositor != NULL, FALSE);

  /* Do not create controller state from the normal input path.  A controller
   * exists only after a real lock transition and from that point onward this
   * must be a complete embargo, including after its client has died. */
  controller = g_object_get_data (G_OBJECT (compositor),
                                  SESSION_LOCK_CONTROLLER_KEY);
  if (!controller || controller->state == META_WAYLAND_SESSION_LOCK_UNLOCKED)
    return FALSE;

  event_type = clutter_event_type (event);
  if (event_type == CLUTTER_KEY_PRESS || event_type == CLUTTER_KEY_RELEASE ||
      event_type == CLUTTER_KEY_STATE || event_type == CLUTTER_IM_COMMIT ||
      event_type == CLUTTER_IM_DELETE || event_type == CLUTTER_IM_PREEDIT)
    {
      /* Keyboard and IM belong to a mapped surface of the active owner. */
      g_hash_table_iter_init (&iter, controller->surfaces);
      while (g_hash_table_iter_next (&iter, NULL, &value))
        {
          MetaWaylandSessionLockSurface *candidate = value;
          if (meta_wayland_session_lock_surface_is_mapped (candidate))
            {
              lock_surface = candidate;
              break;
            }
        }
    }
  else
    {
      float x, y;
      ClutterActor *actor;

      clutter_event_get_coords (event, &x, &y);
      actor = clutter_stage_get_actor_at_pos (controller->stage,
                                              CLUTTER_PICK_ALL, x, y);
      /* Picking can return a texture or subsurface below the Wayland actor.
       * Walk only as far as our private lock scene; never accept a normal
       * stage actor merely because it happens to be beneath the cover. */
      while (actor && actor != controller->scene)
        {
          if (META_IS_SURFACE_ACTOR_WAYLAND (actor))
            {
              surface = meta_surface_actor_wayland_get_surface (
                META_SURFACE_ACTOR_WAYLAND (actor));
              break;
            }
          actor = clutter_actor_get_parent (actor);
        }
      g_hash_table_iter_init (&iter, controller->surfaces);
      while (surface && g_hash_table_iter_next (&iter, NULL, &value))
        {
          MetaWaylandSessionLockSurface *candidate = value;
          if (meta_wayland_session_lock_surface_is_mapped (candidate) &&
              meta_wayland_session_lock_surface_get_wayland_surface (candidate) == surface)
            {
              lock_surface = candidate;
              break;
            }
        }
    }

  if (lock_surface)
    {
      surface = meta_wayland_session_lock_surface_get_wayland_surface (lock_surface);
      /* This bypasses every normal handler and is safe only because surface
       * came from the active controller's output map above. */
      meta_wayland_seat_handle_session_lock_event (compositor->seat, event,
                                                    surface);
    }

  /* A miss (unmapped role, cover, stale actor, unsupported device) remains
   * fail-closed.  Normal-session handlers never see input while active. */
  return TRUE;
}

gulong
meta_wayland_session_lock_add_state_changed_callback (
  MetaWaylandCompositor                    *compositor,
  MetaWaylandSessionLockStateChangedFunc    callback,
  gpointer                                  user_data,
  GDestroyNotify                            destroy_notify)
{
  MetaWaylandSessionLockController *controller;
  MetaWaylandSessionLockStateChangedCallback *entry;
  gulong id;

  g_return_val_if_fail (compositor != NULL, 0);
  g_return_val_if_fail (callback != NULL, 0);

  controller = get_controller (compositor);
  id = ++controller->next_state_changed_callback_id;
  if (id == 0)
    id = ++controller->next_state_changed_callback_id;
  entry = g_new (MetaWaylandSessionLockStateChangedCallback, 1);
  *entry = (MetaWaylandSessionLockStateChangedCallback) {
    .ref_count = 1,
    .callback = callback,
    .user_data = user_data,
    .destroy_notify = destroy_notify,
  };
  g_hash_table_insert (controller->state_changed_callbacks,
                       GSIZE_TO_POINTER (id), entry);
  return id;
}

void
meta_wayland_session_lock_remove_state_changed_callback (
  MetaWaylandCompositor *compositor,
  gulong                 id)
{
  g_return_if_fail (compositor != NULL);

  if (id != 0)
    g_hash_table_remove (get_controller (compositor)->state_changed_callbacks,
                         GSIZE_TO_POINTER (id));
}

gboolean
meta_wayland_session_lock_is_presentation_confirmed (
  MetaWaylandCompositor *compositor)
{
  MetaWaylandSessionLockController *controller;

  g_return_val_if_fail (compositor != NULL, FALSE);

  controller = g_object_get_data (G_OBJECT (compositor),
                                  SESSION_LOCK_CONTROLLER_KEY);
  return controller && controller->presentation_confirmed;
}

ClutterActor *
meta_wayland_session_lock_get_scene (MetaWaylandCompositor *compositor)
{
  MetaWaylandSessionLockController *controller;

  g_return_val_if_fail (compositor != NULL, NULL);

  controller = g_object_get_data (G_OBJECT (compositor),
                                  SESSION_LOCK_CONTROLLER_KEY);
  return controller ? controller->scene : NULL;
}

void
meta_wayland_session_lock_surface_destroyed (
  gpointer                       owner,
  MetaWaylandSessionLockSurface *surface)
{
  MetaWaylandSessionLockController *controller = owner;
  MetaWaylandOutput *output;

  if (!controller)
    return;
  output = meta_wayland_session_lock_surface_get_output (surface);
  if (output && g_hash_table_lookup (controller->surfaces, output) == surface)
    g_hash_table_remove (controller->surfaces, output);
  /* Any destroyed active-output surface leaves only the compositor-owned
   * opaque cover. This is the specified fail-closed fallback. */
}

void
meta_wayland_init_session_lock (MetaWaylandCompositor *compositor)
{
  g_return_if_fail (compositor != NULL);

  /* Unlike established protocol gates, this key defaults off.  Keeping the
   * normal helper's default-true behaviour here would turn an omitted setting
   * into a misleading startup warning on every Gnoblin session. */
  if (gnoblin_config_get_bool ("protocols", "ext-session-lock", FALSE))
    g_warning ("Gnoblin ext-session-lock is requested but remains disabled: "
               "Mutter does not yet provide the required fail-closed scene, "
               "input, output-hotplug, and client-death controller");
}
