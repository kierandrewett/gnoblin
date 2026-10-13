#pragma once
#include <glib-object.h>

/* Boundary data only; routing bodies come verbatim from pinned production. */
typedef enum {
    CLUTTER_NOTHING,
    CLUTTER_KEY_PRESS,
    CLUTTER_KEY_RELEASE,
    CLUTTER_MOTION,
    CLUTTER_BUTTON_PRESS,
    CLUTTER_BUTTON_RELEASE,
    CLUTTER_SCROLL,
    CLUTTER_ENTER,
    CLUTTER_LEAVE,
    CLUTTER_TOUCH_BEGIN,
    CLUTTER_TOUCH_UPDATE,
    CLUTTER_TOUCH_END,
    CLUTTER_TOUCH_CANCEL,
    CLUTTER_TOUCHPAD_PINCH,
    CLUTTER_TOUCHPAD_SWIPE,
    CLUTTER_TOUCHPAD_HOLD,
    CLUTTER_PROXIMITY_IN,
    CLUTTER_PROXIMITY_OUT,
    CLUTTER_PAD_BUTTON_PRESS,
    CLUTTER_PAD_BUTTON_RELEASE,
    CLUTTER_PAD_RING,
    CLUTTER_PAD_STRIP,
    CLUTTER_PAD_DIAL,
    CLUTTER_DEVICE_ADDED,
    CLUTTER_DEVICE_REMOVED,
    CLUTTER_EVENT_LAST
} ClutterEventType;
typedef guint ClutterModifierType;
typedef struct {
    ClutterEventType type;
    float x, y;
    guint button;
} ClutterEvent;
typedef void ClutterActor;
static inline ClutterEventType clutter_event_type(const ClutterEvent* e) {
    return e->type;
}
static inline guint clutter_event_get_button(const ClutterEvent* e) {
    return e->button;
}
static inline void clutter_event_get_coords(const ClutterEvent* e, float* x, float* y) {
    *x = e->x;
    *y = e->y;
}
static inline ClutterModifierType clutter_event_get_state(const ClutterEvent* e) {
    return 0;
}
static inline guint clutter_event_get_key_symbol(const ClutterEvent* e) {
    return 0;
}
static inline gunichar clutter_event_get_key_unicode(const ClutterEvent* e) {
    return 0;
}
#define CLUTTER_BUTTON1_MASK (1u << 8)
#define CLUTTER_BUTTON2_MASK (1u << 9)
#define CLUTTER_BUTTON3_MASK (1u << 10)
#define CLUTTER_EVENT_STOP TRUE
#define CLUTTER_EVENT_PROPAGATE FALSE

G_BEGIN_DECLS
void panel_new(void);
void panel_free(void);
void panel_frame(void);
void panel_dismiss_coords(float* x, float* y);
gboolean panel_visible(void);
gboolean panel_handle(const ClutterEvent* event);
G_END_DECLS
