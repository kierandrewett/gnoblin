/*
 * gnoblin-shell: real content-behind "background blur". See gnoblin-blur.h.
 *
 * The effect's paint() runs at the actor's place in the stage paint order, so by
 * the time it is called the framebuffer already holds everything painted BEHIND
 * the actor (wallpaper + lower windows). We:
 *
 *   1. Read back the framebuffer region the actor covers into a CoglTexture
 *      (the live "content behind" — NOT a scene-graph clone, so no recursion).
 *   2. Blur that texture with a separable Gaussian (two offscreen passes:
 *      horizontal then vertical), spread by the configured radius.
 *   3. Composite the blurred texture back onto the stage at the actor's rect,
 *      masked by the rounded-rectangle SDF so the frost matches the corners.
 *   4. Continue the normal actor paint, so the (translucent) window draws on top
 *      of its own frosted backdrop.
 *
 * GLSL ES 1.00 via CoglSnippet so it runs on llvmpipe (software devkit) too.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-blur.h"

#include <math.h>
#include <string.h>

extern "C" {
#include <clutter/clutter.h>
#include <cogl/cogl.h>
}

/* One separable Gaussian pass, implemented as a FRAGMENT-stage replace snippet
 * (Cogl exposes `cogl_sampler0` + `cogl_tex_coord0_in` here). `blur_dir` selects
 * the axis (1,0 horizontal / 0,1 vertical), `blur_step` is one texel in that
 * axis, `blur_radius` widens the spread. Four taps either side (plus centre)
 * give a smooth frost cheaply enough for the software renderer; weights are a
 * normalised Gaussian. */
static const char* BLUR_PASS_DECL =
    "uniform vec2 blur_dir;\n"
    "uniform float blur_step;\n"
    "uniform float blur_radius;\n";

static const char* BLUR_PASS_SRC =
    "  vec2 tc = cogl_tex_coord0_in.st;\n"
    "  vec2 off = blur_dir * blur_step * max(blur_radius, 0.5);\n"
    "  vec4 sum = texture2D(cogl_sampler0, tc) * 0.20;\n"
    "  float w0 = 0.166; float w1 = 0.111; float w2 = 0.055; float w3 = 0.018;\n"
    "  vec2 d1 = off * 1.4; vec2 d2 = off * 2.8; vec2 d3 = off * 4.2; vec2 d4 = off * 5.6;\n"
    "  sum += texture2D(cogl_sampler0, tc + d1) * w0;\n"
    "  sum += texture2D(cogl_sampler0, tc - d1) * w0;\n"
    "  sum += texture2D(cogl_sampler0, tc + d2) * w1;\n"
    "  sum += texture2D(cogl_sampler0, tc - d2) * w1;\n"
    "  sum += texture2D(cogl_sampler0, tc + d3) * w2;\n"
    "  sum += texture2D(cogl_sampler0, tc - d3) * w2;\n"
    "  sum += texture2D(cogl_sampler0, tc + d4) * w3;\n"
    "  sum += texture2D(cogl_sampler0, tc - d4) * w3;\n"
    "  cogl_color_out = sum;\n";

/* The composite pass: sample the blurred texture and clip it to a rounded-rect
 * SDF so the frost matches the window's corners. `mask_size` is the rect size in
 * px, `mask_radius` the corner radius, `mask_algo`/`mask_smoothing` the corner
 * shape (mirrors gnoblin-rounded). When mask_radius <= 0 the mask is a plain
 * rectangle (full coverage). FRAGMENT replace snippet. */
static const char* MASK_DECL =
    "uniform vec2 mask_size;\n"
    "uniform float mask_radius;\n"
    "uniform int mask_algo;\n"
    "uniform float mask_smoothing;\n"
    "uniform float alpha_threshold;\n" /* frost only where act.a < this cutoff */
    "float gn_sd_circle (vec2 p, vec2 b, float r) {\n"
    "  vec2 q = abs(p) - b + vec2(r);\n"
    "  return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;\n"
    "}\n"
    "float gn_sd_squircle (vec2 p, vec2 b, float r) {\n"
    "  vec2 q = abs(p) - b + vec2(r);\n"
    "  float oe = min(max(q.x, q.y), 0.0);\n"
    "  vec2 m = max(q, vec2(0.0));\n"
    "  float n = 5.0;\n"
    "  float ln = pow(pow(m.x, n) + pow(m.y, n), 1.0 / n);\n"
    "  return oe + ln - r;\n"
    "}\n";

/* Composite the frosted backdrop UNDER the actor: layer 0 is the blurred
 * backdrop, layer 1 is the actor's offscreen render (its alpha tells us where the
 * panel actually has content). We draw `frost` masked by the rounded-rect SDF AND
 * by the actor's coverage, then the actor over the top — so the frost only shows
 * where the (possibly translucent) panel paints, never over the transparent gaps
 * of a full-screen layer surface. Compositing: out = frost*(1-actorA) + actor,
 * with frost premultiplied and clipped to the panel's footprint. */
static const char* MASK_SRC =
    "  vec4 frost = texture2D(cogl_sampler0, cogl_tex_coord0_in.st);\n"
    "  vec4 act = texture2D(cogl_sampler1, cogl_tex_coord1_in.st);\n"
    "  float cover = smoothstep(0.0, 0.04, act.a);\n" /* where the panel has pixels */
    /* Alpha-threshold gate: only frost pixels that are translucent enough — i.e.
     * where the surface's own alpha is below `alpha_threshold`. Near-opaque
     * pixels (act.a >= threshold) show the actor directly with no wasted blur.
     * A small soft band around the cutoff avoids a hard edge. The default
     * threshold (>= 1.0) frosts wherever the panel has any coverage (the historic
     * behaviour, since premultiplied act.a never exceeds 1.0). */
    "  if (alpha_threshold < 1.0) {\n"
    "    cover *= 1.0 - smoothstep(alpha_threshold - 0.04, alpha_threshold, act.a);\n"
    "  }\n"
    "  if (mask_radius > 0.5) {\n"
    "    vec2 px = cogl_tex_coord0_in.st * mask_size;\n"
    "    vec2 c = mask_size * 0.5;\n"
    "    vec2 p = px - c;\n"
    "    float dc = gn_sd_circle(p, c, mask_radius);\n"
    "    float dist = dc;\n"
    "    float blend = (mask_algo == 1) ? max(mask_smoothing, 0.6) : mask_smoothing;\n"
    "    if (blend > 0.001) {\n"
    "      float dsq = gn_sd_squircle(p, c, mask_radius);\n"
    "      dist = mix(dc, dsq, clamp(blend, 0.0, 1.0));\n"
    "    }\n"
    "    cover = min(cover, 1.0 - smoothstep(-1.0, 0.5, dist));\n"
    "  }\n"
    "  frost *= cover;\n"
    "  cogl_color_out = frost * (1.0 - act.a) + act;\n";

typedef struct {
    ClutterOffscreenEffect parent;
    float radius;
    gboolean rounded_set;
    GnoblinRoundedParams rounded;

    /* Frost only where the surface's own alpha is below this cutoff (1.0 = no
     * gating: frost wherever the surface has coverage, the historic behaviour). */
    float alpha_threshold;

    /* The content captured from behind the actor in pre_paint (the live backdrop
     * — wallpaper + any windows underneath), to be blurred in paint_target. */
    CoglTexture* captured;

    /* Cached pipelines (rebuilt lazily). */
    CoglPipeline* blur_h;
    CoglPipeline* blur_v;
    CoglPipeline* composite;
    gboolean pipelines_ready;
} GnoblinBlur;

typedef struct {
    ClutterOffscreenEffectClass parent_class;
} GnoblinBlurClass;

GType gnoblin_blur_get_type(void);
G_DEFINE_TYPE(GnoblinBlur, gnoblin_blur, CLUTTER_TYPE_OFFSCREEN_EFFECT)

#define GNOBLIN_BLUR(o) (G_TYPE_CHECK_INSTANCE_CAST((o), gnoblin_blur_get_type(), GnoblinBlur))

/* A pipeline with one texture layer whose fragment stage is REPLACED by `src`
 * (the body writing cogl_color_out), with `decl` (uniforms + helper functions)
 * emitted at global scope — both on a SINGLE snippet so the declarations are
 * guaranteed to accompany the replace body. */
static CoglPipeline* make_fragment_pipeline(CoglContext* ctx, const char* decl, const char* src) {
    CoglPipeline* p = cogl_pipeline_new(ctx);
    CoglSnippet* snip = cogl_snippet_new(COGL_SNIPPET_HOOK_FRAGMENT, decl, NULL);

    cogl_snippet_set_replace(snip, src);
    cogl_pipeline_add_snippet(p, snip);
    /* Force layer 0 to exist so cogl_sampler0 / cogl_tex_coord0_in are emitted. */
    cogl_pipeline_set_layer_null_texture(p, 0);
    cogl_pipeline_set_layer_filters(p, 0, COGL_PIPELINE_FILTER_LINEAR,
                                    COGL_PIPELINE_FILTER_LINEAR);
    cogl_pipeline_set_layer_wrap_mode(p, 0, COGL_PIPELINE_WRAP_MODE_CLAMP_TO_EDGE);
    g_object_unref(snip);
    return p;
}

static CoglPipeline* make_blur_pipeline(CoglContext* ctx) {
    return make_fragment_pipeline(ctx, BLUR_PASS_DECL, BLUR_PASS_SRC);
}

static CoglPipeline* make_composite_pipeline(CoglContext* ctx) {
    CoglPipeline* p = make_fragment_pipeline(ctx, MASK_DECL, MASK_SRC);
    /* The composite samples a second layer (the actor render) for coverage. */
    cogl_pipeline_set_layer_null_texture(p, 1);
    cogl_pipeline_set_layer_filters(p, 1, COGL_PIPELINE_FILTER_LINEAR,
                                    COGL_PIPELINE_FILTER_LINEAR);
    cogl_pipeline_set_layer_wrap_mode(p, 1, COGL_PIPELINE_WRAP_MODE_CLAMP_TO_EDGE);
    return p;
}

static void ensure_pipelines(GnoblinBlur* self, CoglContext* ctx) {
    if (self->pipelines_ready)
        return;
    self->blur_h = make_blur_pipeline(ctx);
    self->blur_v = make_blur_pipeline(ctx);
    self->composite = make_composite_pipeline(ctx);
    self->pipelines_ready = TRUE;
}

static void set_uniform_1f(CoglPipeline* p, const char* name, float v) {
    int loc = cogl_pipeline_get_uniform_location(p, name);
    if (loc >= 0)
        cogl_pipeline_set_uniform_1f(p, loc, v);
}

static void set_uniform_2f(CoglPipeline* p, const char* name, float a, float b) {
    int loc = cogl_pipeline_get_uniform_location(p, name);
    float v[2] = {a, b};
    if (loc >= 0)
        cogl_pipeline_set_uniform_float(p, loc, 2, 1, v);
}

static void set_uniform_1i(CoglPipeline* p, const char* name, int v) {
    int loc = cogl_pipeline_get_uniform_location(p, name);
    if (loc >= 0)
        cogl_pipeline_set_uniform_1i(p, loc, v);
}

/* Run one separable Gaussian pass src->dst (both `w`x`h`), horizontal if `dir`
 * is (1,0), vertical if (0,1). The destination offscreen gets a 1:1 ortho so a
 * (0,0)-(w,h) quad fills it. */
static void blur_pass(CoglPipeline* pipe, CoglTexture* src, CoglFramebuffer* dst, int w, int h,
                      float dir_x, float dir_y, float radius) {
    cogl_pipeline_set_layer_texture(pipe, 0, src);
    set_uniform_2f(pipe, "blur_dir", dir_x, dir_y);
    set_uniform_1f(pipe, "blur_step", 1.0f / (float)(dir_x != 0.0f ? MAX(w, 1) : MAX(h, 1)));
    set_uniform_1f(pipe, "blur_radius", radius);

    cogl_framebuffer_set_viewport(dst, 0, 0, w, h);
    cogl_framebuffer_orthographic(dst, 0, 0, w, h, -1.0f, 1.0f);
    cogl_framebuffer_draw_textured_rectangle(dst, pipe, 0.0f, 0.0f, (float)w, (float)h, 0.0f, 0.0f,
                                             1.0f, 1.0f);
}

/* pre_paint: capture the live content BEHIND the actor before the offscreen
 * effect redirects the actor's paint elsewhere. At this point the framebuffer is
 * the real destination and already holds the wallpaper + any windows underneath.
 * We read back the actor's rect into `captured`, then chain to the parent so the
 * actor renders into the offscreen FBO (whose texture we get in paint_target). */
static gboolean gnoblin_blur_pre_paint(ClutterEffect* effect, ClutterPaintNode* node,
                                       ClutterPaintContext* paint_context) {
    GnoblinBlur* self = GNOBLIN_BLUR(effect);
    ClutterActor* actor = clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(effect));
    CoglFramebuffer* fb = clutter_paint_context_get_framebuffer(paint_context);
    CoglContext* ctx = fb ? cogl_framebuffer_get_context(fb) : NULL;
    float tx = 0, ty = 0, aw = 0, ah = 0, scale = 1.0f;
    int fb_w, fb_h, rx, ry, rw, rh;

    g_clear_object(&self->captured);

    if (actor && fb && ctx) {
        clutter_actor_get_size(actor, &aw, &ah);
        clutter_actor_get_transformed_position(actor, &tx, &ty);
        scale = clutter_actor_get_resource_scale(actor);
        if (scale <= 0.0f)
            scale = 1.0f;

        fb_w = cogl_framebuffer_get_width(fb);
        fb_h = cogl_framebuffer_get_height(fb);

        /* Actor rect in framebuffer device pixels (top-left origin). Clamp to the
         * framebuffer for partially off-screen surfaces. */
        rx = (int)floorf(tx * scale);
        ry = (int)floorf(ty * scale);
        rw = (int)ceilf(aw * scale);
        rh = (int)ceilf(ah * scale);
        if (rx < 0) {
            rw += rx;
            rx = 0;
        }
        if (ry < 0) {
            rh += ry;
            ry = 0;
        }
        if (rx + rw > fb_w)
            rw = fb_w - rx;
        if (ry + rh > fb_h)
            rh = fb_h - ry;

        if (rw >= 2 && rh >= 2 && aw >= 1 && ah >= 1) {
            CoglBitmap* bmp =
                cogl_bitmap_new_with_size(ctx, rw, rh, COGL_PIXEL_FORMAT_RGBA_8888_PRE);
            if (bmp) {
                if (cogl_framebuffer_read_pixels_into_bitmap(fb, rx, ry,
                                                             COGL_READ_PIXELS_COLOR_BUFFER, bmp)) {
                    self->captured = cogl_texture_2d_new_from_bitmap(bmp);
                    if (self->captured)
                        cogl_texture_set_premultiplied(self->captured, TRUE);
                }
                g_object_unref(bmp);
            }
        }
    }

    return CLUTTER_EFFECT_CLASS(gnoblin_blur_parent_class)
        ->pre_paint(effect, node, paint_context);
}

/* paint_target: the actor has been rendered into the offscreen texture. Blur the
 * captured backdrop, then composite frost (clipped to the rounded-rect AND the
 * actor's coverage) under the actor texture, all in one masked draw. */
static void gnoblin_blur_paint_target(ClutterOffscreenEffect* effect, ClutterPaintNode* node,
                                      ClutterPaintContext* paint_context) {
    GnoblinBlur* self = GNOBLIN_BLUR(effect);
    CoglTexture* actor_tex = clutter_offscreen_effect_get_texture(effect);
    CoglFramebuffer* fb = clutter_paint_context_get_framebuffer(paint_context);
    CoglContext* ctx = fb ? cogl_framebuffer_get_context(fb) : NULL;
    int tw, th;
    CoglTexture* tex_a = NULL;
    CoglTexture* tex_b = NULL;
    CoglOffscreen* off_a = NULL;
    CoglOffscreen* off_b = NULL;

    /* No captured backdrop (off-screen, or read-back failed): fall back to the
     * plain actor paint so nothing disappears. */
    if (!self->captured || !actor_tex || !fb || !ctx) {
        CLUTTER_OFFSCREEN_EFFECT_CLASS(gnoblin_blur_parent_class)
            ->paint_target(effect, node, paint_context);
        return;
    }

    tw = cogl_texture_get_width(self->captured);
    th = cogl_texture_get_height(self->captured);
    ensure_pipelines(self, ctx);

    tex_a = cogl_texture_2d_new_with_size(ctx, tw, th);
    tex_b = cogl_texture_2d_new_with_size(ctx, tw, th);
    if (!tex_a || !tex_b) {
        g_clear_object(&tex_a);
        g_clear_object(&tex_b);
        CLUTTER_OFFSCREEN_EFFECT_CLASS(gnoblin_blur_parent_class)
            ->paint_target(effect, node, paint_context);
        return;
    }
    off_a = cogl_offscreen_new_with_texture(tex_a);
    off_b = cogl_offscreen_new_with_texture(tex_b);

    /* Separable Gaussian: captured -> tex_a (H) -> tex_b (V). */
    blur_pass(self->blur_h, self->captured, COGL_FRAMEBUFFER(off_a), tw, th, 1.0f, 0.0f,
              self->radius);
    blur_pass(self->blur_v, tex_a, COGL_FRAMEBUFFER(off_b), tw, th, 0.0f, 1.0f, self->radius);

    /* Composite onto the destination with the current modelview (which maps the
     * offscreen actor texture to its on-screen footprint). Layer 0 = blurred
     * backdrop, layer 1 = actor render. The shader masks the frost by the
     * rounded-rect SDF and the actor's own coverage, then puts the actor over. */
    cogl_pipeline_set_layer_texture(self->composite, 0, tex_b);
    cogl_pipeline_set_layer_texture(self->composite, 1, actor_tex);
    set_uniform_2f(self->composite, "mask_size", (float)tw, (float)th);
    set_uniform_1f(self->composite, "alpha_threshold", self->alpha_threshold);
    if (self->rounded_set && self->rounded.radius > 0.0f) {
        float rscale = clutter_actor_get_resource_scale(
            clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(effect)));
        if (rscale <= 0.0f)
            rscale = 1.0f;
        set_uniform_1f(self->composite, "mask_radius", self->rounded.radius * rscale);
        set_uniform_1i(self->composite, "mask_algo", (int)self->rounded.algorithm);
        set_uniform_1f(self->composite, "mask_smoothing", self->rounded.smoothing);
    } else {
        set_uniform_1f(self->composite, "mask_radius", 0.0f);
        set_uniform_1i(self->composite, "mask_algo", 0);
        set_uniform_1f(self->composite, "mask_smoothing", 0.0f);
    }

    {
        /* Draw the actor-sized quad. The offscreen texture is the actor's paint
         * box; draw it 1:1 in the texture's own pixel size so layers line up. */
        float qw = (float)cogl_texture_get_width(actor_tex);
        float qh = (float)cogl_texture_get_height(actor_tex);
        cogl_framebuffer_draw_textured_rectangle(fb, self->composite, 0.0f, 0.0f, qw, qh, 0.0f,
                                                 0.0f, 1.0f, 1.0f);
    }

    g_clear_object(&off_a);
    g_clear_object(&off_b);
    g_clear_object(&tex_a);
    g_clear_object(&tex_b);
}

static void gnoblin_blur_dispose(GObject* object) {
    GnoblinBlur* self = GNOBLIN_BLUR(object);

    g_clear_object(&self->captured);
    g_clear_object(&self->blur_h);
    g_clear_object(&self->blur_v);
    g_clear_object(&self->composite);
    self->pipelines_ready = FALSE;

    G_OBJECT_CLASS(gnoblin_blur_parent_class)->dispose(object);
}

static void gnoblin_blur_class_init(GnoblinBlurClass* klass) {
    G_OBJECT_CLASS(klass)->dispose = gnoblin_blur_dispose;
    CLUTTER_EFFECT_CLASS(klass)->pre_paint = gnoblin_blur_pre_paint;
    CLUTTER_OFFSCREEN_EFFECT_CLASS(klass)->paint_target = gnoblin_blur_paint_target;
}

static void gnoblin_blur_init(GnoblinBlur* self) {
    self->radius = 24.0f;
    self->rounded_set = FALSE;
    self->pipelines_ready = FALSE;
    self->alpha_threshold = 1.0f; /* no gating by default (frost all coverage) */
}

ClutterEffect* gnoblin_blur_new(float radius) {
    GnoblinBlur* self = (GnoblinBlur*)g_object_new(gnoblin_blur_get_type(), NULL);

    self->radius = radius;
    return CLUTTER_EFFECT(self);
}

void gnoblin_blur_set_alpha_threshold(ClutterEffect* effect, float threshold) {
    GnoblinBlur* self;

    if (!effect || !G_TYPE_CHECK_INSTANCE_TYPE(effect, gnoblin_blur_get_type()))
        return;
    self = GNOBLIN_BLUR(effect);
    /* Clamp to [0,1]; >= 1.0 means "frost everywhere the surface has coverage". */
    self->alpha_threshold = CLAMP(threshold, 0.0f, 1.0f);
}

void gnoblin_blur_set_rounded(ClutterEffect* effect, const GnoblinRoundedParams* params) {
    GnoblinBlur* self;

    if (!effect || !G_TYPE_CHECK_INSTANCE_TYPE(effect, gnoblin_blur_get_type()))
        return;
    self = GNOBLIN_BLUR(effect);
    if (params) {
        self->rounded = *params;
        self->rounded_set = TRUE;
    } else {
        self->rounded_set = FALSE;
    }
}
