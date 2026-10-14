#pragma once

#include <glib.h>
#include "gnoblin-vr-view.h"
#include "gnoblin-vr-vk.h"

/* Turns the scene into draw items for the renderer. */

/* Corner radius for window quads, in metres. */
#define VR_WINDOW_RADIUS 0.02f

/* Appends one item per surface that has pixels, lowest stacking rank first. */
void vr_compose_surfaces(const VrView* view, const VrPose* head, GArray* items);

/* A thin ribbon from @origin along @direction for @length metres, turned to face @eye. */
void vr_compose_ray(GArray* items, const char* key, const double origin[3],
                    const double direction[3], double length, const double eye[3],
                    const guint8 premultiplied_rgba[4], float width);

typedef struct {
    guint8* pixels; /* premultiplied BGRA */
    guint32 width;
    guint32 height;
    gint hot_x;
    gint hot_y;
    guint64 serial;
} VrCursor;

/* Loads the compositor's cursor sprite (cursor.rgba and cursor.ini) when its serial changed.
 * Returns TRUE when the sprite is usable. */
gboolean vr_cursor_refresh(VrCursor* cursor, const char* directory);
void vr_cursor_clear(VrCursor* cursor);
/* A plain round pointer for sessions where the compositor publishes no sprite. */
void vr_cursor_default(VrCursor* cursor);

/* A cursor billboard whose hotspot sits on @point, facing @eye, sized to keep a constant angular
 * size with distance. @up is the head's up vector. */
void vr_compose_cursor(GArray* items, const VrCursor* cursor, const double point[3],
                       const double eye[3], const double up[3]);

/* The flat colour used for rays, as a premultiplied BGRA pixel. */
void vr_compose_solid_pixel(guint8 out[4], guint8 red, guint8 green, guint8 blue, guint8 alpha);
