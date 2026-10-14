#pragma once

#include <gio/gio.h>

/* Compositor-side spatial scene for the optional VR extension. It stores anchors and
 * surface placements submitted by gnoblin.spatial and answers snapshot queries. Rendering,
 * texture import and hit testing consume this store; they are not part of it. */

/* Handles a vr.spatial.* method. Returns a floating-or-full-ref a{sv} result, or NULL
 * with @error set. Methods: anchor, place, attach, move, raise, update_pose, surface,
 * snapshot. */
GVariant* gnoblin_vr_scene_call(const char* method, GVariant* arguments, GError** error);

/* Drops every anchor and surface and restarts revisions; used on session end and tests. */
void gnoblin_vr_scene_reset(void);
