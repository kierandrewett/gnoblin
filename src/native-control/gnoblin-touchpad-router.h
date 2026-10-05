/* Pure touchpad gesture recognition for the native Gnoblin runtime. */
#pragma once

#include <glib.h>

G_BEGIN_DECLS

typedef struct _GnoblinTouchpadRouter GnoblinTouchpadRouter;

GnoblinTouchpadRouter* gnoblin_touchpad_router_new(void);
void gnoblin_touchpad_router_free(GnoblinTouchpadRouter* router);
void gnoblin_touchpad_router_reset(GnoblinTouchpadRouter* router);

G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinTouchpadRouter, gnoblin_touchpad_router_free)

/*
 * Handle one compositor gesture payload.  @gestures is the configured av of
 * boxed vardicts and @payload is a compositor vardict.  A non-NULL
 * @matched_gesture receives an owned vardict only for a successful end.
 */
gboolean gnoblin_touchpad_router_handle(GnoblinTouchpadRouter* router, GVariant* gestures,
                                        GVariant* payload, const char* context,
                                        GVariant** matched_gesture);

G_END_DECLS
