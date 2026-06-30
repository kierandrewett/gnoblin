/*
 * gnoblin-shell: CSS box-shadow parsing for compositor drop shadows.
 *
 * Kept separate from the Clutter effect so the config surface can be tested
 * without starting a compositor.
 */

#pragma once

#include <glib.h>

#define GNOBLIN_SHADOW_MAX_LAYERS 4

G_BEGIN_DECLS

/* One box-shadow layer: pixel offsets, blur radius, spread radius, and an RGBA
 * colour (components 0..1; the alpha is that layer's peak opacity). */
typedef struct {
    float offset_x;
    float offset_y;
    float blur;
    float spread;
    float color[4];
} GnoblinShadowLayer;

gboolean gnoblin_shadow_parse_css_color(const char* s, float out[4]);
int gnoblin_shadow_parse_box_shadow(const char* css, GnoblinShadowLayer* layers, int max);
float gnoblin_shadow_pad_for_layers(const GnoblinShadowLayer* layers, int count);

G_END_DECLS
