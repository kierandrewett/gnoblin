/*
 * gnoblin-shell: rounded window corners as a ClutterShaderEffect. See
 * gnoblin-rounded.h.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-rounded.h"

#include <string.h>

extern "C" {
#include <clutter/clutter.h>
}

/* The fragment shader masks the actor's corners with a rounded-rectangle SDF and
 * optionally strokes a border along that edge. `tex` is the offscreen-rendered
 * actor (texture unit 0), `size` its pixel dimensions, `radius` the corner
 * radius. Multiplying by cogl_color_in keeps actor opacity (so open/close fades
 * still apply).
 *
 * Corner shape: at `algo == 0` we use the exact circular rounded-box SDF. At
 * `algo == 1` we use a superellipse / squircle distance. `smoothing` (0..1)
 * blends linearly between the two distance fields, so the corner reads anywhere
 * from a pure circle (smoothing 0) to a strong continuous-corner squircle
 * (smoothing 1) — the same control Figma exposes as "corner smoothing".
 *
 * Border: `border_w > 0` draws a stroke band `border_w` px wide on the inside of
 * the rounded edge (so it never grows the actor). `border_style == 1` is a flat
 * line in `border_col`. `border_style == 2` is the macOS "raised lip": a light
 * inner highlight that is brighter toward the TOP of the window and fades down
 * the sides, plus a faint dark outer hairline — together a subtle 3D bevel.
 * GLSL ES 1.00 (no fancy builtins); smoothstep stands in for AA. */
static const char* ROUNDED_SHADER =
    "uniform sampler2D tex;\n"
    "uniform vec2 size;\n"
    "uniform float radius;\n"
    "uniform int algo;\n"
    "uniform float smoothing;\n"
    "uniform float border_w;\n"
    "uniform int border_style;\n"
    "uniform vec4 border_col;\n"
    "uniform float ring_w;\n"   /* RING style: outer ring thickness */
    "uniform vec4 ring_col;\n"  /* RING style: outer ring colour (focus picked CPU-side) */
    /* Per-side inset (px) from the texture edge to the ACTUAL window surface:
     * (left, top, right, bottom). CSD apps (GTK/firefox/nautilus) reserve an
     * invisible shadow border inside their buffer, so the visible window is
     * smaller than the actor; this insets the rounded edge + border/ring so they
     * hug the real surface instead of floating out past the shadow. Zero for SSD
     * windows where the buffer already equals the frame. */
    "uniform vec4 inset;\n"
    /* When 1, fill any transparency the client left inside our rounded silhouette
     * (a self-rounding app's own rounded corners) with its edge colour, so there
     * is no gap between the client's corner and ours. */
    "uniform int corner_fill;\n"
    /* Circular rounded-box SDF (negative inside). */
    "float sd_circle (vec2 p, vec2 b, float r) {\n"
    "  vec2 q = abs(p) - b + vec2(r);\n"
    "  return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;\n"
    "}\n"
    /* Superellipse / squircle SDF approximation. Inside the straight edges it is
     * identical to the box; only within the corner quadrant do we swap the L2
     * corner distance for an Ln (superellipse) one with exponent ~4 (iOS-ish).
     * Returned as a signed distance so it composes like the circular field. */
    "float sd_squircle (vec2 p, vec2 b, float r) {\n"
    "  vec2 q = abs(p) - b + vec2(r);\n"
    "  float outside_edge = min(max(q.x, q.y), 0.0);\n"
    "  vec2 m = max(q, vec2(0.0));\n"
    /* Exponent ~5 gives the iOS/macOS continuous-corner look: the corner stays
     * fuller for longer then turns more sharply than a circle, so the radius
     * reads larger while the silhouette is unmistakably a squircle. */
    "  float n = 5.0;\n"
    "  float ln = pow(pow(m.x, n) + pow(m.y, n), 1.0 / n);\n"
    "  return outside_edge + ln - r;\n"
    "}\n"
    "void main () {\n"
    "  vec2 uv = cogl_tex_coord_in[0].xy;\n"
    "  vec2 px = uv * size;\n"
    /* The content rect is the texture inset per side by `inset` (left,top,right,
     * bottom) — the visible window surface inside any CSD shadow margin. Round
     * and stroke THAT rect, not the full texture. */
    "  vec2 c0 = vec2(inset.x, inset.y);\n"
    "  vec2 c1 = size - vec2(inset.z, inset.w);\n"
    "  vec2 cc = (c0 + c1) * 0.5;\n"
    "  vec2 p = px - cc;\n"
    "  vec2 b = (c1 - c0) * 0.5;\n"
    "  float r = radius;\n"
    "  float d_circle = sd_circle(p, b, r);\n"
    "  float dist = d_circle;\n"
    /* `blend` is how far toward the squircle field we go (0 = pure circle).
     * algo==1 (squircle) treats `smoothing` as strength with a sensible floor so
     * "squircle" always visibly differs from a circle even at smoothing 0; algo==0
     * (circle) honours `smoothing` directly as a circle->squircle blend. */
    "  float blend = (algo == 1) ? max(smoothing, 0.6) : smoothing;\n"
    "  if (blend > 0.001) {\n"
    "    float d_sq = sd_squircle(p, b, r);\n"
    "    dist = mix(d_circle, d_sq, clamp(blend, 0.0, 1.0));\n"
    "  }\n"
    "  float alpha = 1.0 - smoothstep(-1.0, 0.5, dist);\n"
    "  vec4 src = texture2D(tex, uv);\n"
    /* Smart corner fill: sample the window's own edge colour just inside the
     * rounded rect (clamp the pixel into the inner box by the radius) and
     * composite the possibly-transparent corner pixel OVER it. A self-rounding
     * client's transparent corner is thus filled with its edge colour instead of
     * leaving a gap inside our rounded silhouette. Premultiplied "over". */
    "  if (corner_fill == 1) {\n"
    "    vec2 fpx = clamp(px, c0 + vec2(r + 1.0), c1 - vec2(r + 1.0));\n"
    "    vec4 fillc = texture2D(tex, fpx / size);\n"
    "    src = src + fillc * (1.0 - src.a);\n"
    "  }\n"
    "  vec4 base = cogl_color_in * src * alpha;\n"
    /* RING: a Tailwind `border` + `ring` rendered as two crisp ~1px bands stacked
     * INSIDE the rounded edge (like `border` + `ring-inset` box-shadows). From the
     * edge inward: `border` (the light layer), then `ring` (the dark layer). Both
     * sharp (no soft falloff — a box-shadow ring is a crisp line), clipped to the
     * mask and premultiplied; focused/unfocused colours chosen CPU-side. */
    "  if (border_style == 3) {\n"
    "    float cedge = -dist;\n"                          /* depth inside the content */
    "    float bw = max(border_w, 0.0);\n"
    "    float rw = max(ring_w, 0.0);\n"
    "    float aa = 0.6;\n"                               /* edge softness in px */
    /* border: the outermost band, cedge in [0, bw]. */
    "    float border_band = (1.0 - smoothstep(bw - aa, bw + aa, cedge)) * alpha;\n"
    /* ring: inset just inside the border, cedge in [bw, bw+rw]. */
    "    float ring_band = smoothstep(bw - aa, bw + aa, cedge)\n"
    "                    * (1.0 - smoothstep(bw + rw - aa, bw + rw + aa, cedge)) * alpha;\n"
    "    vec4 bc = border_col; bc.rgb *= bc.a;\n"
    "    base.rgb = mix(base.rgb, bc.rgb, border_band);\n"
    "    base.a   = mix(base.a, base.a * (1.0 - border_col.a) + border_col.a, border_band);\n"
    "    vec4 rc = ring_col; rc.rgb *= rc.a;\n"
    "    base.rgb = mix(base.rgb, rc.rgb, ring_band);\n"
    "    base.a   = mix(base.a, base.a * (1.0 - ring_col.a) + ring_col.a, ring_band);\n"
    "  } else if (border_style != 0 && border_w > 0.5) {\n"
    /* Band: from the rounded edge inward by border_w. inner = how far inside the
     * border band we are (1 at the very edge -> 0 at border_w deep). */
    "    float edge = -dist;\n"                      /* >0 inside the shape */
    "    float band = 1.0 - smoothstep(border_w - 1.0, border_w + 0.5, edge);\n"
    "    band *= alpha;\n"                            /* clip the band to the mask */
    "    if (border_style == 1) {\n"
    "      vec4 bc = border_col;\n"
    "      bc.rgb *= bc.a;\n"                          /* premultiply to match base */
    "      base.rgb = mix(base.rgb, bc.rgb, band);\n"
    "      base.a   = mix(base.a, base.a * (1.0 - bc.a) + bc.a, band);\n"
    "    } else {\n"
    /* Raised lip: a light inner highlight whose strength depends on vertical
     * position (brightest at the top, faint at the bottom), plus a thin darker
     * outer hairline at the very edge. ny: -1 top .. +1 bottom. */
    "      float ny = p.y / max(b.y, 1.0);\n"
    "      float top_bias = clamp(0.5 - ny * 0.5, 0.15, 1.0);\n"
    "      float hl = band * top_bias;\n"             /* inner highlight coverage */
    "      float outer = 1.0 - smoothstep(1.0, 2.5, edge);\n" /* very edge only */
    "      outer *= alpha;\n"
    "      vec3 light = vec3(1.0);\n"
    "      vec3 dark = border_col.rgb;\n"
    "      float la = hl * 0.45;\n"                    /* highlight opacity */
    "      base.rgb = mix(base.rgb, light * base.a, la);\n"
    "      float da = outer * 0.35 * border_col.a;\n" /* outer hairline opacity */
    "      base.rgb = mix(base.rgb, dark * base.a, da);\n"
    "    }\n"
    "  }\n"
    "  cogl_color_out = base;\n"
    "}\n";

typedef struct {
    ClutterShaderEffect parent;
    GnoblinRoundedParams params;
    gboolean source_set;
    gboolean focused; /* RING: pick the focused vs unfocused colours */
} GnoblinRounded;

typedef struct {
    ClutterShaderEffectClass parent_class;
} GnoblinRoundedClass;

GType gnoblin_rounded_get_type(void);
G_DEFINE_TYPE(GnoblinRounded, gnoblin_rounded, CLUTTER_TYPE_SHADER_EFFECT)

#define GNOBLIN_ROUNDED(o)                                                                         \
    (G_TYPE_CHECK_INSTANCE_CAST((o), gnoblin_rounded_get_type(), GnoblinRounded))

static void gnoblin_rounded_paint_target(ClutterOffscreenEffect* effect, ClutterPaintNode* node,
                                         ClutterPaintContext* paint_context) {
    GnoblinRounded* self = GNOBLIN_ROUNDED(effect);
    ClutterShaderEffect* shader = CLUTTER_SHADER_EFFECT(effect);
    ClutterActor* actor = clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(effect));
    float width = 0, height = 0;
    double scale_x = 1.0, scale_y = 1.0;
    float radius = self->params.radius;
    float border_w = self->params.border_width;
    float ring_w = self->params.ring_width;

    float res_scale = 1.0f;
    if (actor) {
        clutter_actor_get_scale(actor, &scale_x, &scale_y);
        res_scale = (float)clutter_actor_get_resource_scale(actor);
        if (!(res_scale > 0.0f))
            res_scale = 1.0f;
    }

    /* Size the rounded rect to the actual offscreen TEXTURE — the window's full
     * paint box in physical px — NOT the actor allocation. SSD windows draw their
     * titlebar/borders as child surfaces that extend BEYOND the allocation (e.g.
     * the titlebar sits 26px above it), so the allocation is smaller than the
     * painted window; rounding the allocation leaves the ring floating outside
     * the real edge. The offscreen texture is exactly what gets painted, so it is
     * the correct silhouette. (CSD/normal windows: texture == allocation, no
     * change.) */
    CoglTexture* tex = clutter_offscreen_effect_get_texture(CLUTTER_OFFSCREEN_EFFECT(effect));
    if (tex) {
        width = (float)cogl_texture_get_width(tex);
        height = (float)cogl_texture_get_height(tex);
    } else if (actor) {
        clutter_actor_get_size(actor, &width, &height);
        width *= res_scale;
        height *= res_scale;
    }

    if (!self->source_set) {
        clutter_shader_effect_set_shader_source(shader, ROUNDED_SHADER);
        self->source_set = TRUE;
    }

    /* radius/border/ring are configured in LOGICAL px; the texture is in PHYSICAL
     * px, so scale them by the resource (output/HiDPI) scale. Also keep them
     * on-screen-constant while a transform-scale animation (maximize/restore)
     * grows the actor, by dividing out that transform scale. */
    {
        double avg = (scale_x > 0.0001 && scale_y > 0.0001) ? (scale_x + scale_y) * 0.5 : 1.0;
        float k = res_scale / (float)avg;
        radius = self->params.radius * k;
        border_w = self->params.border_width * k;
        ring_w = self->params.ring_width * k;
    }

    /* Per-side inset to the visible surface (CSD shadow margin), in LOGICAL px →
     * scale to the texture's physical px. 0 for SSD (round the whole paint box). */
    float in[4] = {self->params.content_inset[0] * res_scale,
                   self->params.content_inset[1] * res_scale,
                   self->params.content_inset[2] * res_scale,
                   self->params.content_inset[3] * res_scale};

    /* Uniforms float-collected as doubles via varargs (see mutter's conform
     * tests); the sampler is texture unit 0. */
    clutter_shader_effect_set_uniform(shader, "tex", G_TYPE_INT, 1, 0);
    clutter_shader_effect_set_uniform(shader, "size", G_TYPE_FLOAT, 2, (double)width,
                                      (double)height);
    clutter_shader_effect_set_uniform(shader, "inset", G_TYPE_FLOAT, 4, (double)in[0],
                                      (double)in[1], (double)in[2], (double)in[3]);
    clutter_shader_effect_set_uniform(shader, "corner_fill", G_TYPE_INT, 1,
                                      self->params.corner_fill ? 1 : 0);
    clutter_shader_effect_set_uniform(shader, "radius", G_TYPE_FLOAT, 1, (double)radius);
    clutter_shader_effect_set_uniform(shader, "algo", G_TYPE_INT, 1,
                                      (int)self->params.algorithm);
    clutter_shader_effect_set_uniform(shader, "smoothing", G_TYPE_FLOAT, 1,
                                      (double)self->params.smoothing);
    clutter_shader_effect_set_uniform(shader, "border_w", G_TYPE_FLOAT, 1, (double)border_w);
    clutter_shader_effect_set_uniform(shader, "border_style", G_TYPE_INT, 1,
                                      (int)self->params.border_style);
    /* RING picks the focused/unfocused colours; LINE/LIP ignore focus. */
    const float* bcol = (self->params.border_style == GNOBLIN_BORDER_RING && self->focused)
                            ? self->params.border_color_focused
                            : self->params.border_color;
    const float* rcol =
        self->focused ? self->params.ring_color_focused : self->params.ring_color;
    clutter_shader_effect_set_uniform(shader, "border_col", G_TYPE_FLOAT, 4, (double)bcol[0],
                                      (double)bcol[1], (double)bcol[2], (double)bcol[3]);
    clutter_shader_effect_set_uniform(shader, "ring_w", G_TYPE_FLOAT, 1, (double)ring_w);
    clutter_shader_effect_set_uniform(shader, "ring_col", G_TYPE_FLOAT, 4, (double)rcol[0],
                                      (double)rcol[1], (double)rcol[2], (double)rcol[3]);

    CLUTTER_OFFSCREEN_EFFECT_CLASS(gnoblin_rounded_parent_class)
        ->paint_target(effect, node, paint_context);
}

static void gnoblin_rounded_class_init(GnoblinRoundedClass* klass) {
    CLUTTER_OFFSCREEN_EFFECT_CLASS(klass)->paint_target = gnoblin_rounded_paint_target;
}

static void gnoblin_rounded_init(GnoblinRounded* self) {
    self->params.radius = 12.0f;
    self->params.algorithm = GNOBLIN_ROUNDED_CIRCLE;
    self->params.smoothing = 0.0f;
    self->params.border_style = GNOBLIN_BORDER_NONE;
    self->params.border_width = 0.0f;
    self->params.border_color[0] = self->params.border_color[1] = self->params.border_color[2] =
        0.0f;
    self->params.border_color[3] = 0.0f;
    self->params.ring_width = 0.0f;
    memset(self->params.ring_color, 0, sizeof(self->params.ring_color));
    memset(self->params.border_color_focused, 0, sizeof(self->params.border_color_focused));
    memset(self->params.ring_color_focused, 0, sizeof(self->params.ring_color_focused));
    memset(self->params.content_inset, 0, sizeof(self->params.content_inset));
    self->params.corner_fill = FALSE;
    self->source_set = FALSE;
    self->focused = FALSE;
}

ClutterEffect* gnoblin_rounded_new_full(const GnoblinRoundedParams* params) {
    GnoblinRounded* self = (GnoblinRounded*)g_object_new(gnoblin_rounded_get_type(), NULL);

    if (params)
        self->params = *params;
    return CLUTTER_EFFECT(self);
}

void gnoblin_rounded_set_focused(ClutterEffect* effect, gboolean focused) {
    GnoblinRounded* self;

    if (!effect || !G_TYPE_CHECK_INSTANCE_TYPE(effect, gnoblin_rounded_get_type()))
        return;
    self = GNOBLIN_ROUNDED(effect);
    if (self->focused == focused)
        return;
    self->focused = focused;
    /* Repaint so the new colours show (only matters for RING). */
    if (self->params.border_style == GNOBLIN_BORDER_RING) {
        ClutterActor* actor = clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(effect));
        if (actor)
            clutter_actor_queue_redraw(actor);
    }
}

ClutterEffect* gnoblin_rounded_new(float radius) {
    GnoblinRoundedParams params;

    memset(&params, 0, sizeof(params));
    params.radius = radius;
    params.algorithm = GNOBLIN_ROUNDED_CIRCLE;
    return gnoblin_rounded_new_full(&params);
}
