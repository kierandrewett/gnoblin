/*
 * Gnoblin session-lock protocol boundary.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include "clutter/clutter.h"
#include "wayland/meta-wayland-types.h"

typedef enum
{
  META_WAYLAND_SESSION_LOCK_UNLOCKED,
  META_WAYLAND_SESSION_LOCK_COVERING,
  META_WAYLAND_SESSION_LOCK_LOCKED,
  META_WAYLAND_SESSION_LOCK_FAILSAFE,
} MetaWaylandSessionLockState;

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

/* Policy consumers (capture, clipboard, remote input) must deny access from
 * COVERING onward.  This stays true after a locker crash in FAILSAFE. */
gboolean
meta_wayland_session_lock_is_active (MetaWaylandCompositor *compositor);

/* True only after a compositor-owned covered frame reached every current
 * stage view. Suspend coordinators must not treat COVERING as confirmation. */
gboolean
meta_wayland_session_lock_is_presentation_confirmed (MetaWaylandCompositor *compositor);

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
