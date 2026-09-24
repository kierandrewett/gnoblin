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
#include "clutter/clutter.h"
#include "core/meta-context-private.h"
#include "wayland/meta-wayland-data-device-primary.h"
#include "wayland/meta-wayland-data-device.h"
#include "wayland/meta-wayland-input.h"
#include "wayland/meta-wayland-keyboard.h"
#include "wayland/meta-wayland-pointer.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-seat.h"
#include "wayland/meta-wayland-session-lock.h"
#include "wayland/meta-wayland-touch.h"

#include "wayland/gnoblin-config.h"

/* This data stays on the compositor object, whose lifetime owns its stage and
 * seat.  It is deliberately separate from the ext-session-lock resource: a
 * client may disappear, whereas the cover and input embargo must survive. */
typedef struct
{
  MetaWaylandSessionLockState state;
  ClutterActor *cover;
  MetaWaylandEventHandler *input_handler;
} MetaWaylandSessionLockController;

#define SESSION_LOCK_CONTROLLER_KEY "gnoblin-session-lock-controller"

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
  meta_wayland_keyboard_set_focus (seat->keyboard, NULL);
  meta_wayland_pointer_focus_surface (seat->pointer, NULL);
  meta_wayland_touch_cancel (seat->touch);
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
destroy_controller (gpointer data)
{
  MetaWaylandSessionLockController *controller = data;

  if (controller->cover)
    clutter_actor_destroy (controller->cover);

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

  /* Appending to the stage puts the opaque actor above Mutter's window, top
   * window, feedback, and Shell actors.  The stage actor is painted on every
   * view, so a newly added or resized output is covered without a client
   * round-trip. */
  controller->cover = clutter_actor_new ();
  clutter_actor_set_background_color (controller->cover,
                                      &COGL_COLOR_INIT (0, 0, 0, 255));
  clutter_actor_set_size (controller->cover,
                          clutter_actor_get_width (stage),
                          clutter_actor_get_height (stage));
  clutter_actor_set_reactive (controller->cover, FALSE);
  clutter_actor_add_child (stage, controller->cover);

  controller->input_handler =
    meta_wayland_input_attach_event_handler (compositor->seat->input_handler,
                                              &failsafe_input_interface,
                                              TRUE,
                                              compositor);
  controller->state = META_WAYLAND_SESSION_LOCK_FAILSAFE;
  clutter_actor_queue_redraw (stage);
}

MetaWaylandSessionLockState
meta_wayland_session_lock_get_state (MetaWaylandCompositor *compositor)
{
  g_return_val_if_fail (compositor != NULL,
                        META_WAYLAND_SESSION_LOCK_FAILSAFE);

  return get_controller (compositor)->state;
}

void
meta_wayland_init_session_lock (MetaWaylandCompositor *compositor)
{
  g_return_if_fail (compositor != NULL);

  if (gnoblin_config_protocol_enabled ("ext-session-lock"))
    g_warning ("Gnoblin ext-session-lock is requested but remains disabled: "
               "Mutter does not yet provide the required fail-closed scene, "
               "input, output-hotplug, and client-death controller");
}
