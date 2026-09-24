/*
 * Gnoblin session-lock protocol boundary.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <sys/types.h>

#include "clutter/clutter.h"
#include "wayland/meta-wayland-types.h"

typedef enum
{
  META_WAYLAND_SESSION_LOCK_UNLOCKED,
  META_WAYLAND_SESSION_LOCK_COVERING,
  META_WAYLAND_SESSION_LOCK_LOCKED,
  META_WAYLAND_SESSION_LOCK_FAILSAFE,
} MetaWaylandSessionLockState;

/* Private compositor observers use this to revoke sensitive services at the
 * same instant that a session transitions into its covered state. */
typedef void (*MetaWaylandSessionLockStateChangedFunc) (
  MetaWaylandCompositor      *compositor,
  MetaWaylandSessionLockState state,
  gpointer                    user_data);

/*
 * Enter the compositor-owned fail-safe state.  This function is internal to
 * Mutter and is intentionally not connected to a client protocol yet.
 *
 * It installs an opaque top-stage cover and a grabbing Wayland input handler.
 * The operation is idempotent; once active it can only be left through the
 * eventual owner-validated unlock path.
 */
void meta_wayland_session_lock_enter_failsafe (MetaWaylandCompositor *compositor);

MetaWaylandSessionLockState
meta_wayland_session_lock_get_state (MetaWaylandCompositor *compositor);

/* Zero until the standard manager global is actually registered after all
 * scene, input and capture gates are complete. */
guint
meta_wayland_session_lock_get_capability (MetaWaylandCompositor *compositor);

/* Policy consumers (capture, clipboard, remote input) must deny access from
 * COVERING onward.  This stays true after a locker crash in FAILSAFE. */
gboolean
meta_wayland_session_lock_is_active (MetaWaylandCompositor *compositor);

/*
 * First event-path embargo.  Call this before any input consumers which can
 * invoke shortcuts, input methods, accessibility clients, or gestures.  It
 * returns TRUE whenever the compositor owns a lock state, including the
 * client-death fail-safe state; callers must then stop processing the event.
 *
 * This is deliberately a policy gate rather than a client callback.  The
 * eventual verified lock-surface dispatcher runs behind this gate and may
 * only receive events for controller-owned, mapped lock roles.
 */
gboolean
meta_wayland_session_lock_filter_event (MetaWaylandCompositor *compositor,
                                        const ClutterEvent     *event);

/* True only after a compositor-owned covered frame reached every current
 * stage view. Suspend coordinators must not treat COVERING as confirmation. */
gboolean
meta_wayland_session_lock_is_presentation_confirmed (MetaWaylandCompositor *compositor);

/* Callbacks run synchronously on Mutter's main context after every state
 * transition. Consumers must treat every state other than UNLOCKED as denied. */
gulong
meta_wayland_session_lock_add_state_changed_callback (
  MetaWaylandCompositor                    *compositor,
  MetaWaylandSessionLockStateChangedFunc    callback,
  gpointer                                  user_data,
  GDestroyNotify                            destroy_notify);

void
meta_wayland_session_lock_remove_state_changed_callback (
  MetaWaylandCompositor *compositor,
  gulong                 id);

/* Internal scene parent for lock-surface actors. It is NULL until the
 * fail-safe cover has been installed. Normal clients must never use it. */
ClutterActor *
meta_wayland_session_lock_get_scene (MetaWaylandCompositor *compositor);

/*
 * Register the session-lock implementation boundary.
 *
 * This currently never creates ext_session_lock_manager_v1.  Advertising a
 * session lock manager before the compositor has a fail-closed scene and
 * input controller would falsely claim a security guarantee.
 */
void meta_wayland_init_session_lock (MetaWaylandCompositor *compositor);
