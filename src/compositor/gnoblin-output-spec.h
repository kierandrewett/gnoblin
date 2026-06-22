/*
 * gnoblin-shell: strict parsing for [output] monitor layout specs.
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

typedef struct {
    gboolean disable;
    gboolean primary;
    gboolean has_mode, has_scale, has_position, has_transform;
    int mode_w, mode_h;
    double mode_refresh; /* 0 = any */
    double scale;
    int pos_x, pos_y;
    guint transform;
} GnoblinOutputSpec;

gboolean gnoblin_output_transform_from_name(const char* text, guint* out);
gboolean gnoblin_output_parse_spec(const char* value, GnoblinOutputSpec* out);

G_END_DECLS
