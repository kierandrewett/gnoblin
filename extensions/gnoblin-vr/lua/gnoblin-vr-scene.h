#pragma once

#include <gio/gio.h>

/* Compositor-side spatial scene for the optional VR extension. It stores anchors and
 * surface placements submitted by gnoblin.spatial and answers snapshot queries. Rendering,
 * texture import and hit testing consume this store; they are not part of it. */

/* Handles a vr.spatial.* method. Returns a floating-or-full-ref a{sv} result, or NULL
 * with @error set. Methods: anchor, place, attach, move, raise, update_pose, surface,
 * snapshot. */
GVariant* gnoblin_vr_scene_call(const char* method, GVariant* arguments, GError** error);

/* The headset as an outputs record (kind "xr"), read from the status file that gnoblin-vr
 * writes at $XDG_RUNTIME_DIR/gnoblin-vr/status.ini. Returns a full-ref a{sv}, or NULL when the
 * extension has never reported. The record has per-eye "views" and no logical rectangle. */
GVariant* gnoblin_vr_output_record(void);

/* Drops every anchor and surface and restarts revisions; used on session end and tests. */
void gnoblin_vr_scene_reset(void);
