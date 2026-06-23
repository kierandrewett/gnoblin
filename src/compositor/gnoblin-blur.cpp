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
    "uniform vec2 frost_off;\n"        /* actor footprint origin in the padded capture */
    "uniform vec2 frost_scale;\n"      /* actor footprint size in the padded capture */
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
    "  vec4 frost = texture2D(cogl_sampler0, frost_off + cogl_tex_coord0_in.st * frost_scale);\n"
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

static void enlarge_box_for_effects(ClutterActorBox* box) {
    float width, height;

    if (!box || clutter_actor_box_get_area(box) == 0.0f)
        return;

    /* Mirrors Clutter's private _clutter_actor_box_enlarge_for_effects().
     * ClutterOffscreenEffect uses this enlarged box for the FBO, then offsets
     * the final texture back by box.x1/box.y1. This effect paints its own
     * target, so it must use the same actor sub-rect inside that FBO. */
    width = box->x2 - box->x1;
    height = box->y2 - box->y1;
    width = nearbyintf(width);
    height = nearbyintf(height);

    box->x2 = ceilf(box->x2 + 0.75f);
    box->y2 = ceilf(box->y2 + 0.75f);
    box->x1 = box->x2 - width - 3.0f;
    box->y1 = box->y2 - height - 3.0f;
}

static void get_actor_offscreen_rect(ClutterActor* actor, CoglTexture* actor_tex, float* x1,
                                     float* y1, float* x2, float* y2) {
    float width = 0.0f, height = 0.0f;
    float scale = 1.0f;
    int tex_w = actor_tex ? cogl_texture_get_width(actor_tex) : 1;
    int tex_h = actor_tex ? cogl_texture_get_height(actor_tex) : 1;
    float inset_x = 0.0f, inset_y = 0.0f;
    const ClutterPaintVolume* volume = NULL;

    if (actor)
        clutter_actor_get_size(actor, &width, &height);

    if (actor) {
        scale = clutter_actor_get_resource_scale(actor);
        if (scale <= 0.0f)
            scale = 1.0f;
        volume = clutter_actor_get_paint_volume(actor);
    }

    if (volume) {
        graphene_point3d_t origin;
        ClutterActorBox box;

        clutter_paint_volume_get_origin(volume, &origin);
        box.x1 = origin.x;
        box.y1 = origin.y;
        box.x2 = origin.x + clutter_paint_volume_get_width(volume);
        box.y2 = origin.y + clutter_paint_volume_get_height(volume);
        enlarge_box_for_effects(&box);

        inset_x = MAX((0.0f - box.x1) * scale, 0.0f);
        inset_y = MAX((0.0f - box.y1) * scale, 0.0f);
    }

    *x1 = CLAMP(inset_x, 0.0f, (float)tex_w);
    *y1 = CLAMP(inset_y, 0.0f, (float)tex_h);
    *x2 = CLAMP(inset_x + width * scale, *x1, (float)tex_w);
    *y2 = CLAMP(inset_y + height * scale, *y1, (float)tex_h);
}

typedef struct {
    ClutterOffscreenEffect parent;
    float radius;
    gboolean rounded_set;
    GnoblinRoundedParams rounded;

    /* Frost only where the surface's own alpha is below this cutoff (1.0 = no
     * gating: frost wherever the surface has coverage, the historic behaviour). */
    float alpha_threshold;

    /* The content captured from behind the actor in pre_paint (the live backdrop
     * — wallpaper + any windows underneath), to be blurred in paint_target. The
     * capture is PADDED by ~the blur radius beyond the actor's rect so the
     * Gaussian has real backdrop to sample at the edges instead of smearing the
     * clamped edge texel into a halo. `frost_off`/`frost_scale` map the actor's
     * 0..1 footprint to its sub-rect inside the padded capture. */
    CoglTexture* captured;
    float frost_off[2];
    float frost_scale[2];

    /* Previous frame's blurred backdrop, for temporal smoothing. The read-back
     * blur feeds back on itself through double-buffering (each buffer captures
     * its OWN frost from its previous use), making the frost TOGGLE between two
     * states = flicker. Averaging this frame's blur with the last converges both
     * buffers to a common value and kills the toggle. Sized sw x sh; reset when
     * the low-res dimensions change (geometry/radius change). */
    CoglTexture* history;
    int hist_w;
    int hist_h;
    int hist_cap_x; /* capture origin the history corresponds to */
    int hist_cap_y;
    int cap_x; /* this frame's capture origin (set in pre_paint) */
    int cap_y;

    /* Cached pipelines (rebuilt lazily). */
    CoglPipeline* downsample;
    CoglPipeline* blur_h;
    CoglPipeline* blur_v;
    CoglPipeline* temporal;
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

/* Temporal mix: layer 0 = this frame's blur, layer 1 = last frame's. Output is
 * their average, so the double-buffer feedback toggle averages to a stable
 * value (kills the flicker). 0.5 = a 2-frame box average. */
static const char* TEMPORAL_SRC =
    "  vec4 a = texture2D(cogl_sampler0, cogl_tex_coord0_in.st);\n"
    "  vec4 b = texture2D(cogl_sampler1, cogl_tex_coord1_in.st);\n"
    "  cogl_color_out = mix(b, a, 0.5);\n";

static CoglPipeline* make_temporal_pipeline(CoglContext* ctx) {
    CoglPipeline* p = make_fragment_pipeline(ctx, "", TEMPORAL_SRC);
    cogl_pipeline_set_layer_null_texture(p, 1);
    cogl_pipeline_set_layer_filters(p, 1, COGL_PIPELINE_FILTER_LINEAR,
                                    COGL_PIPELINE_FILTER_LINEAR);
    cogl_pipeline_set_layer_wrap_mode(p, 1, COGL_PIPELINE_WRAP_MODE_CLAMP_TO_EDGE);
    return p;
}

/* A plain textured pipeline (linear filter) used to bilinearly downsample the
 * captured backdrop before blurring — halving the resolution averages 2x2 blocks
 * so the cheap 9-tap Gaussian covers the full radius smoothly instead of
 * undersampling it into a smear. */
static CoglPipeline* make_copy_pipeline(CoglContext* ctx) {
    CoglPipeline* p = cogl_pipeline_new(ctx);
    cogl_pipeline_set_layer_null_texture(p, 0);
    cogl_pipeline_set_layer_filters(p, 0, COGL_PIPELINE_FILTER_LINEAR, COGL_PIPELINE_FILTER_LINEAR);
    cogl_pipeline_set_layer_wrap_mode(p, 0, COGL_PIPELINE_WRAP_MODE_CLAMP_TO_EDGE);
    return p;
}

static void ensure_pipelines(GnoblinBlur* self, CoglContext* ctx) {
    if (self->pipelines_ready)
        return;
    self->downsample = make_copy_pipeline(ctx);
    self->blur_h = make_blur_pipeline(ctx);
    self->blur_v = make_blur_pipeline(ctx);
    self->temporal = make_temporal_pipeline(ctx);
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
    self->frost_off[0] = self->frost_off[1] = 0.0f;
    self->frost_scale[0] = self->frost_scale[1] = 1.0f;

    if (actor && fb && ctx) {
        int ax, ay, aw_i, ah_i, pad, cx0, cy0, cx1, cy1, cw, ch;

        clutter_actor_get_size(actor, &aw, &ah);
        clutter_actor_get_transformed_position(actor, &tx, &ty);
        scale = clutter_actor_get_resource_scale(actor);
        if (scale <= 0.0f)
            scale = 1.0f;

        fb_w = cogl_framebuffer_get_width(fb);
        fb_h = cogl_framebuffer_get_height(fb);

        /* Actor rect in framebuffer device pixels (top-left origin), clamped to
         * the framebuffer for partially off-screen surfaces. */
        rx = (int)floorf(tx * scale);
        ry = (int)floorf(ty * scale);
        rw = (int)ceilf(aw * scale);
        rh = (int)ceilf(ah * scale);
        ax = rx < 0 ? 0 : rx;
        ay = ry < 0 ? 0 : ry;
        aw_i = (rx + rw > fb_w ? fb_w : rx + rw) - ax;
        ah_i = (ry + rh > fb_h ? fb_h : ry + rh) - ay;

        /* Capture PADDED by ~1.5x the blur radius so the Gaussian has real
         * backdrop to sample at the actor's edges (no clamp-to-edge halo). The
         * pad is clamped per-side to the framebuffer; the actor footprint within
         * the captured texture is recorded for the composite to remap. */
        pad = (int)ceilf(self->radius * scale * 1.5f) + 2;
        cx0 = ax - pad < 0 ? 0 : ax - pad;
        cy0 = ay - pad < 0 ? 0 : ay - pad;
        cx1 = ax + aw_i + pad > fb_w ? fb_w : ax + aw_i + pad;
        cy1 = ay + ah_i + pad > fb_h ? fb_h : ay + ah_i + pad;
        cw = cx1 - cx0;
        ch = cy1 - cy0;

        if (cw >= 2 && ch >= 2 && aw_i >= 1 && ah_i >= 1) {
            CoglBitmap* bmp =
                cogl_bitmap_new_with_size(ctx, cw, ch, COGL_PIXEL_FORMAT_RGBA_8888_PRE);
            if (bmp) {
                if (cogl_framebuffer_read_pixels_into_bitmap(fb, cx0, cy0,
                                                             COGL_READ_PIXELS_COLOR_BUFFER, bmp)) {
                    self->captured = cogl_texture_2d_new_from_bitmap(bmp);
                    if (self->captured) {
                        cogl_texture_set_premultiplied(self->captured, TRUE);
                        self->frost_off[0] = (float)(ax - cx0) / (float)cw;
                        self->frost_off[1] = (float)(ay - cy0) / (float)ch;
                        self->frost_scale[0] = (float)aw_i / (float)cw;
                        self->frost_scale[1] = (float)ah_i / (float)ch;
                        self->cap_x = cx0; /* for temporal-smoothing geometry check */
                        self->cap_y = cy0;
                    }
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
    ClutterActor* actor = clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(effect));
    CoglTexture* actor_tex = clutter_offscreen_effect_get_texture(effect);
    CoglFramebuffer* fb = clutter_paint_context_get_framebuffer(paint_context);
    CoglContext* ctx = fb ? cogl_framebuffer_get_context(fb) : NULL;
    int cw, ch, sw, sh;
    CoglTexture* tex_half = NULL;
    CoglTexture* tex_a = NULL;
    CoglTexture* tex_b = NULL;
    CoglOffscreen* off_half = NULL;
    CoglOffscreen* off_a = NULL;
    CoglOffscreen* off_b = NULL;
    CoglTexture* smoothed = NULL;
    CoglTexture* frost = NULL;

    /* No captured backdrop (off-screen, or read-back failed): fall back to the
     * plain actor paint so nothing disappears. */
    if (!self->captured || !actor_tex || !fb || !ctx) {
        CLUTTER_OFFSCREEN_EFFECT_CLASS(gnoblin_blur_parent_class)
            ->paint_target(effect, node, paint_context);
        return;
    }

    cw = cogl_texture_get_width(self->captured);
    ch = cogl_texture_get_height(self->captured);
    ensure_pipelines(self, ctx);

    /* Downsample by a factor that SCALES WITH THE RADIUS, so the fixed 9-tap
     * Gaussian always operates on a small (~5-texel) radius in the low-res grid.
     * The old fixed half-res left the taps tens of texels apart at radius 32 —
     * 9 samples spread over ~90 texels, which under-samples into a streaky SMEAR
     * instead of a smooth blur. With f ~= radius/5 the kernel's farthest tap
     * (5.6 texels low-res) maps back to ~the requested radius in real pixels. */
    {
        int f = (int)lroundf(self->radius / 5.0f);
        f = CLAMP(f, 1, 12);
        sw = MAX(cw / f, 1);
        sh = MAX(ch / f, 1);
    }
    tex_half = cogl_texture_2d_new_with_size(ctx, sw, sh);
    tex_a = cogl_texture_2d_new_with_size(ctx, sw, sh);
    tex_b = cogl_texture_2d_new_with_size(ctx, sw, sh);
    if (!tex_half || !tex_a || !tex_b) {
        g_clear_object(&tex_half);
        g_clear_object(&tex_a);
        g_clear_object(&tex_b);
        CLUTTER_OFFSCREEN_EFFECT_CLASS(gnoblin_blur_parent_class)
            ->paint_target(effect, node, paint_context);
        return;
    }
    off_half = cogl_offscreen_new_with_texture(tex_half);
    off_a = cogl_offscreen_new_with_texture(tex_a);
    off_b = cogl_offscreen_new_with_texture(tex_b);

    /* Downsample captured -> tex_half (bilinear average). */
    cogl_pipeline_set_layer_texture(self->downsample, 0, self->captured);
    cogl_framebuffer_set_viewport(COGL_FRAMEBUFFER(off_half), 0, 0, sw, sh);
    cogl_framebuffer_orthographic(COGL_FRAMEBUFFER(off_half), 0, 0, sw, sh, -1.0f, 1.0f);
    cogl_framebuffer_draw_textured_rectangle(COGL_FRAMEBUFFER(off_half), self->downsample, 0.0f,
                                             0.0f, (float)sw, (float)sh, 0.0f, 0.0f, 1.0f, 1.0f);

    /* Separable Gaussian on the half-res backdrop: tex_half -> tex_a (H) ->
     * tex_b (V). Radius halved to match the half-res grid. */
    /* Tight kernel: ~1 texel between taps in the low-res grid (the downsample
     * above already encodes the real-world radius), so the 9 taps form a smooth
     * Gaussian instead of a sparse smear. */
    blur_pass(self->blur_h, tex_half, COGL_FRAMEBUFFER(off_a), sw, sh, 1.0f, 0.0f, 1.2f);
    blur_pass(self->blur_v, tex_a, COGL_FRAMEBUFFER(off_b), sw, sh, 0.0f, 1.0f, 1.2f);

    /* Temporal smoothing: average this frame's blur (tex_b) with the previous
     * frame's, which converges the two double-buffered frost states to a common
     * value and kills the flicker toggle. Reset when the low-res size changes. */
    frost = tex_b;
    if (self->history && self->hist_w == sw && self->hist_h == sh &&
        self->hist_cap_x == self->cap_x && self->hist_cap_y == self->cap_y) {
        smoothed = cogl_texture_2d_new_with_size(ctx, sw, sh);
        if (smoothed) {
            CoglOffscreen* off_sm = cogl_offscreen_new_with_texture(smoothed);
            cogl_pipeline_set_layer_texture(self->temporal, 0, tex_b);
            cogl_pipeline_set_layer_texture(self->temporal, 1, self->history);
            cogl_framebuffer_set_viewport(COGL_FRAMEBUFFER(off_sm), 0, 0, sw, sh);
            cogl_framebuffer_orthographic(COGL_FRAMEBUFFER(off_sm), 0, 0, sw, sh, -1.0f, 1.0f);
            cogl_framebuffer_draw_textured_rectangle(COGL_FRAMEBUFFER(off_sm), self->temporal, 0.0f,
                                                     0.0f, (float)sw, (float)sh, 0.0f, 0.0f, 1.0f,
                                                     1.0f);
            g_object_unref(off_sm);
            frost = smoothed;
        }
    }
    /* Stash the frost as next frame's history (own a ref). */
    g_clear_object(&self->history);
    self->history = (CoglTexture*)g_object_ref(frost);
    self->hist_w = sw;
    self->hist_h = sh;
    self->hist_cap_x = self->cap_x;
    self->hist_cap_y = self->cap_y;

    /* Composite onto the destination with the current modelview (which maps the
     * offscreen actor texture to its on-screen footprint). Layer 0 = blurred
     * backdrop (remapped to the actor footprint within the padded capture),
     * layer 1 = actor render. The shader masks the frost by the rounded-rect SDF
     * and the actor's own coverage, then puts the actor over. */
    float rect_x1, rect_y1, rect_x2, rect_y2;
    get_actor_offscreen_rect(actor, actor_tex, &rect_x1, &rect_y1, &rect_x2, &rect_y2);

    cogl_pipeline_set_layer_texture(self->composite, 0, frost);
    cogl_pipeline_set_layer_texture(self->composite, 1, actor_tex);
    set_uniform_2f(self->composite, "mask_size", (float)cogl_texture_get_width(actor_tex),
                   (float)cogl_texture_get_height(actor_tex));
    set_uniform_2f(self->composite, "frost_off", self->frost_off[0], self->frost_off[1]);
    set_uniform_2f(self->composite, "frost_scale", self->frost_scale[0], self->frost_scale[1]);
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
        float tx = 1.0f, ty = 1.0f;
        float right_pad = MAX((float)cogl_texture_get_width(actor_tex) - rect_x2, 0.0f);
        float bottom_pad = MAX((float)cogl_texture_get_height(actor_tex) - rect_y2, 0.0f);
        float qw = rect_x2 + right_pad;
        float qh = rect_y2 + bottom_pad;
        float qx1 = 0.0f;
        float qy1 = 0.0f;

        if (actor)
            clutter_actor_get_transformed_position(actor, &tx, &ty);
        if (tx <= 0.5f)
            qx1 = -rect_x1;
        if (ty <= 0.5f)
            qy1 = -rect_y1;

        cogl_framebuffer_draw_textured_rectangle(fb, self->composite, qx1, qy1, qx1 + qw, qy1 + qh,
                                                 0.0f, 0.0f, 1.0f, 1.0f);
    }

    g_clear_object(&off_half);
    g_clear_object(&off_a);
    g_clear_object(&off_b);
    g_clear_object(&tex_half);
    g_clear_object(&tex_a);
    g_clear_object(&tex_b);
    g_clear_object(&smoothed); /* history holds its own ref now */
}

static void gnoblin_blur_dispose(GObject* object) {
    GnoblinBlur* self = GNOBLIN_BLUR(object);

    g_clear_object(&self->captured);
    g_clear_object(&self->history);
    g_clear_object(&self->downsample);
    g_clear_object(&self->blur_h);
    g_clear_object(&self->blur_v);
    g_clear_object(&self->temporal);
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
    self->frost_off[0] = self->frost_off[1] = 0.0f;
    self->frost_scale[0] = self->frost_scale[1] = 1.0f;
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
