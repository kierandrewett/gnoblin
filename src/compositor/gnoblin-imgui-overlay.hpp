/*
 * Gnoblin compositor overlay backed by Dear ImGui and Cogl.
 *
 * This interface deliberately owns no UI policy.  The compositor creates one
 * overlay actor and panels register a draw callback.  This keeps recovery,
 * developer tooling, and future shell diagnostics in the compositor process
 * without adding another executable or toolkit dependency.
 */

#pragma once

#include <clutter/clutter.h>

typedef struct ImGuiContext ImGuiContext;
typedef struct _GnoblinImGuiOverlay GnoblinImGuiOverlay;

G_BEGIN_DECLS

/* Called during the overlay actor's paint. The current ImGui context is the
 * overlay's context for the duration of this callback. */
typedef void (*GnoblinImGuiOverlayDrawFunc) (GnoblinImGuiOverlay *overlay,
                                             void                 *user_data);

/* The actor is attached to the compositor stage when @stage is supplied.
 * Callers own the returned overlay and release it with free(). */
GnoblinImGuiOverlay *gnoblin_imgui_overlay_new (ClutterActor *stage);
void gnoblin_imgui_overlay_free (GnoblinImGuiOverlay *overlay);

ClutterActor *gnoblin_imgui_overlay_get_actor (GnoblinImGuiOverlay *overlay);
ImGuiContext *gnoblin_imgui_overlay_get_context (GnoblinImGuiOverlay *overlay);

void gnoblin_imgui_overlay_set_draw_callback (GnoblinImGuiOverlay        *overlay,
                                              GnoblinImGuiOverlayDrawFunc callback,
                                              void                        *user_data,
                                              GDestroyNotify                destroy);
void gnoblin_imgui_overlay_set_visible (GnoblinImGuiOverlay *overlay,
                                        gboolean             visible);
gboolean gnoblin_imgui_overlay_is_visible (GnoblinImGuiOverlay *overlay);
/* Keep the visible diagnostic actor as the final stage child. */
void gnoblin_imgui_overlay_raise (GnoblinImGuiOverlay *overlay);
void gnoblin_imgui_overlay_queue_redraw (GnoblinImGuiOverlay *overlay);

/* Input is supplied by the compositor's event routing code. Coordinates are
 * actor-local logical pixels. The renderer never creates an input surface. */
void gnoblin_imgui_overlay_set_pointer (GnoblinImGuiOverlay *overlay,
                                        float                x,
                                        float                y,
                                        gboolean             left_down,
                                        gboolean             right_down,
                                        gboolean             middle_down);
void gnoblin_imgui_overlay_add_text (GnoblinImGuiOverlay *overlay,
                                     const char           *utf8);

G_END_DECLS
