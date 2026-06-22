/*
 * gnoblin-shell: soft, multi-layer window drop shadows as a ClutterShaderEffect.
 *
 * See gnoblin-shadow.h. Up to four CSS-`box-shadow` layers are composited (each
 * an offset + blur + spread + rgba). The per-layer coverage is `1 -
 * smoothstep(-blur, blur, d)` over the rounded-rect SDF distance `d`: 50% at
 * the spread edge, fading to 0 at `blur` px out — an erf-like soft penumbra.
 * Layers are composited front-to-back in premultiplied alpha.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-shadow.h"

#include <string.h>

extern "C" {
#include <clutter/clutter.h>
}

/* The actor is `pad` px larger than the window on every side; the window
 * contains the buffer plus pad; the visible frame can be inset inside the
 * buffer when a toolkit uses transparent margins/CSD shadows. `tex` is unused
 * (the actor has no content) but ClutterShaderEffect always binds it. GLSL ES
 * 1.00: no tanh, so smoothstep stands in for the error function. */
static const char* SHADOW_SHADER =
    "uniform sampler2D tex;\n"
    "uniform vec2 size;\n"
    "uniform float pad;\n"
    "uniform float radius;\n"
    "uniform vec4 margins;\n"
    "uniform int count;\n"
    "uniform vec2 off0;\nuniform float blur0;\nuniform float spread0;\nuniform vec4 col0;\n"
    "uniform vec2 off1;\nuniform float blur1;\nuniform float spread1;\nuniform vec4 col1;\n"
    "uniform vec2 off2;\nuniform float blur2;\nuniform float spread2;\nuniform vec4 col2;\n"
    "uniform vec2 off3;\nuniform float blur3;\nuniform float spread3;\nuniform vec4 col3;\n"
    "float sd_round_box (vec2 p, vec2 b, float r) {\n"
    "  vec2 q = abs(p) - b + vec2(r);\n"
    "  return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;\n"
    "}\n"
    "float cov (vec2 p, vec2 hs, float r, vec2 off, float blur, float spread) {\n"
    "  vec2 shadow_hs = max(hs + vec2(spread), vec2(0.0));\n"
    "  float shadow_r = max(r + spread, 0.0);\n"
    "  float d = sd_round_box(p - off, shadow_hs, shadow_r);\n"
    "  return 1.0 - smoothstep(-blur, blur, d);\n"
    "}\n"
    "void main () {\n"
    "  vec2 buffer_size = max(size - vec2(2.0 * pad), vec2(1.0));\n"
    "  vec2 frame_size = max(buffer_size - vec2(margins.x + margins.z, margins.y + margins.w), vec2(1.0));\n"
    "  vec2 frame_origin = vec2(pad + margins.x, pad + margins.y);\n"
    "  vec2 frame_center = frame_origin + frame_size * 0.5;\n"
    "  vec2 p = cogl_tex_coord_in[0].xy * size - frame_center;\n"
    "  vec2 hs = frame_size * 0.5;\n"
    "  vec3 rgb = vec3(0.0);\n"
    "  float a = 0.0;\n"
    "  float la;\n"
    "  if (count > 0) { la = col0.a * cov(p, hs, radius, off0, blur0, spread0);\n"
    "    rgb = col0.rgb * la + rgb * (1.0 - la); a = la + a * (1.0 - la); }\n"
    "  if (count > 1) { la = col1.a * cov(p, hs, radius, off1, blur1, spread1);\n"
    "    rgb = col1.rgb * la + rgb * (1.0 - la); a = la + a * (1.0 - la); }\n"
    "  if (count > 2) { la = col2.a * cov(p, hs, radius, off2, blur2, spread2);\n"
    "    rgb = col2.rgb * la + rgb * (1.0 - la); a = la + a * (1.0 - la); }\n"
    "  if (count > 3) { la = col3.a * cov(p, hs, radius, off3, blur3, spread3);\n"
    "    rgb = col3.rgb * la + rgb * (1.0 - la); a = la + a * (1.0 - la); }\n"
    "  cogl_color_out = vec4(rgb, a) * cogl_color_in.a;\n"
    "}\n";

typedef struct {
    ClutterShaderEffect parent;
    float pad;
    float radius;
    float margins[4];
    GnoblinShadowLayer layers[GNOBLIN_SHADOW_MAX_LAYERS];
    int count;
    gboolean source_set;
} GnoblinShadow;

typedef struct {
    ClutterShaderEffectClass parent_class;
} GnoblinShadowClass;

GType gnoblin_shadow_get_type(void);
G_DEFINE_TYPE(GnoblinShadow, gnoblin_shadow, CLUTTER_TYPE_SHADER_EFFECT)

#define GNOBLIN_SHADOW(o)                                                                          \
    (G_TYPE_CHECK_INSTANCE_CAST((o), gnoblin_shadow_get_type(), GnoblinShadow))

static void set_layer_uniforms(ClutterShaderEffect* shader, int i, const GnoblinShadowLayer* l) {
    const char* off_names[] = {"off0", "off1", "off2", "off3"};
    const char* blur_names[] = {"blur0", "blur1", "blur2", "blur3"};
    const char* spread_names[] = {"spread0", "spread1", "spread2", "spread3"};
    const char* col_names[] = {"col0", "col1", "col2", "col3"};

    clutter_shader_effect_set_uniform(shader, off_names[i], G_TYPE_FLOAT, 2, (double)l->offset_x,
                                      (double)l->offset_y);
    clutter_shader_effect_set_uniform(shader, blur_names[i], G_TYPE_FLOAT, 1, (double)l->blur);
    clutter_shader_effect_set_uniform(shader, spread_names[i], G_TYPE_FLOAT, 1,
                                      (double)l->spread);
    clutter_shader_effect_set_uniform(shader, col_names[i], G_TYPE_FLOAT, 4, (double)l->color[0],
                                      (double)l->color[1], (double)l->color[2],
                                      (double)l->color[3]);
}

static void gnoblin_shadow_paint_target(ClutterOffscreenEffect* effect, ClutterPaintNode* node,
                                        ClutterPaintContext* paint_context) {
    GnoblinShadow* self = GNOBLIN_SHADOW(effect);
    ClutterShaderEffect* shader = CLUTTER_SHADER_EFFECT(effect);
    ClutterActor* actor = clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(effect));
    float width = 0, height = 0;
    GnoblinShadowLayer zero = {0, 0, 0, 0, {0, 0, 0, 0}};
    int i;

    if (actor)
        clutter_actor_get_size(actor, &width, &height);

    if (!self->source_set) {
        clutter_shader_effect_set_shader_source(shader, SHADOW_SHADER);
        self->source_set = TRUE;
    }

    clutter_shader_effect_set_uniform(shader, "tex", G_TYPE_INT, 1, 0);
    clutter_shader_effect_set_uniform(shader, "size", G_TYPE_FLOAT, 2, (double)width,
                                      (double)height);
    clutter_shader_effect_set_uniform(shader, "pad", G_TYPE_FLOAT, 1, (double)self->pad);
    clutter_shader_effect_set_uniform(shader, "radius", G_TYPE_FLOAT, 1, (double)self->radius);
    clutter_shader_effect_set_uniform(shader, "margins", G_TYPE_FLOAT, 4,
                                      (double)self->margins[0], (double)self->margins[1],
                                      (double)self->margins[2], (double)self->margins[3]);
    clutter_shader_effect_set_uniform(shader, "count", G_TYPE_INT, 1, self->count);

    for (i = 0; i < GNOBLIN_SHADOW_MAX_LAYERS; i++)
        set_layer_uniforms(shader, i, i < self->count ? &self->layers[i] : &zero);

    CLUTTER_OFFSCREEN_EFFECT_CLASS(gnoblin_shadow_parent_class)
        ->paint_target(effect, node, paint_context);
}

static void gnoblin_shadow_class_init(GnoblinShadowClass* klass) {
    CLUTTER_OFFSCREEN_EFFECT_CLASS(klass)->paint_target = gnoblin_shadow_paint_target;
}

static void gnoblin_shadow_init(GnoblinShadow* self) {
    self->pad = 48.0f;
    self->radius = 12.0f;
    self->margins[0] = self->margins[1] = self->margins[2] = self->margins[3] = 0.0f;
    self->count = 0;
    self->source_set = FALSE;
}

ClutterEffect* gnoblin_shadow_new(float pad, float radius, float frame_margin_left,
                                  float frame_margin_top, float frame_margin_right,
                                  float frame_margin_bottom,
                                  const GnoblinShadowLayer* layers, int count) {
    GnoblinShadow* self = (GnoblinShadow*)g_object_new(gnoblin_shadow_get_type(), NULL);
    int i;

    self->pad = pad;
    self->radius = radius;
    self->margins[0] = MAX(frame_margin_left, 0.0f);
    self->margins[1] = MAX(frame_margin_top, 0.0f);
    self->margins[2] = MAX(frame_margin_right, 0.0f);
    self->margins[3] = MAX(frame_margin_bottom, 0.0f);
    self->count = CLAMP(count, 0, GNOBLIN_SHADOW_MAX_LAYERS);
    for (i = 0; i < self->count; i++)
        self->layers[i] = layers[i];
    return CLUTTER_EFFECT(self);
}
