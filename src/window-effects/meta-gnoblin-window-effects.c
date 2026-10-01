/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Compositor-owned visual effects for managed client surface actors. */
#include "config.h"
#include "compositor/meta-gnoblin-window-effects.h"

#include <cogl/cogl.h>
#include <math.h>
#include <string.h>

#include "meta/meta-shaped-texture.h"
#include "meta/meta-window-actor.h"

#define ROUNDED_CLIP_EFFECT_NAME "gnoblin-rounded-clip"
#define ROUNDED_CLIP_PADDING_LIMIT 128.
#define CSD_PROBE_CACHE_KEY "gnoblin-csd-probe-cache"
#define CSD_MAX_PIXELS 16000000

typedef struct {
    ClutterOffscreenEffect parent;
    double radius;
    double exponent;
    gboolean automatic;
    double padding[4];
    gboolean csd_reconstruction;
    double csd_insets[4];
} MetaGnoblinRoundedClip;

typedef struct {
    int width;
    int height;
    gboolean detected;
    guint attempts;
    double insets[4];
} CsdProbeCache;

static void csd_insets_to_actor_units(ClutterActor* actor, int width, int height,
                                      const double pixel_insets[4], double insets[4]) {
    double scale_x = clutter_actor_get_width(actor) / MAX(width, 1);
    double scale_y = clutter_actor_get_height(actor) / MAX(height, 1);
    insets[0] = pixel_insets[0] * scale_y;
    insets[1] = pixel_insets[1] * scale_x;
    insets[2] = pixel_insets[2] * scale_y;
    insets[3] = pixel_insets[3] * scale_x;
}

static guint8 pixel_alpha(const guint8* pixels, int width, int x, int y) {
    return pixels[((gsize)y * width + x) * 4 + 3];
}

static int detect_alpha_edge(const guint8* line, int length, int max_distance, int transition) {
    int limit = MIN(max_distance, length - 4);
    guint8 reference;

    if (limit < 4)
        return -1;
    reference = line[length / 2];
    if (reference < 64)
        return -1;

    for (int i = 0; i < 4; i++) {
        if (ABS((int)line[i] - (int)reference) > 8)
            break;
        if (i == 3)
            return 0;
    }

    for (int i = 1; i < limit; i++) {
        int start = MAX(0, i - transition);
        gboolean stable = TRUE;
        gboolean transparent_prefix = TRUE;

        for (int j = 0; j < 4; j++)
            stable &= ABS((int)line[i + j] - (int)reference) <= 8;
        for (int j = 0; j <= start; j++)
            transparent_prefix &= line[j] < reference * 0.8;
        if ((int)line[i] - (int)line[start] >= 32 && stable && transparent_prefix)
            return i;
    }

    return -1;
}

static void sample_alpha_line(const guint8* pixels, int width, int height, int corner, int axis,
                              int offset, guint8* line, int* length) {
    gboolean right = corner == 1 || corner == 2;
    gboolean bottom = corner >= 2;

    if (axis == 0) {
        *length = width;
        int y = bottom ? height - 1 - offset : offset;
        for (int i = 0; i < width; i++) {
            int x = right ? width - 1 - i : i;
            line[i] = pixel_alpha(pixels, width, x, y);
        }
    } else {
        *length = height;
        int x = right ? width - 1 - offset : offset;
        for (int i = 0; i < height; i++) {
            int y = bottom ? height - 1 - i : i;
            line[i] = pixel_alpha(pixels, width, x, y);
        }
    }
}

static int detect_corner_axis(const guint8* pixels, int width, int height, int corner, int axis,
                              guint8* line) {
    static const int offsets[] = {1, 2, 4, 8};
    int count = 0;
    int maximum = -1;

    for (guint i = 0; i < G_N_ELEMENTS(offsets); i++) {
        int length;
        sample_alpha_line(pixels, width, height, corner, axis, offsets[i], line, &length);
        int inset = detect_alpha_edge(line, length, length >> 1, 8);
        if (inset >= 0) {
            count++;
            maximum = MAX(maximum, inset);
        }
    }

    return count >= 2 ? maximum : -1;
}

static gboolean detect_csd_insets(const guint8* pixels, int width, int height, double insets[4]) {
    g_autofree guint8* line = g_malloc(MAX(width, height));
    int corner[4][2];
    int radius[4];

    for (int c = 0; c < 4; c++) {
        corner[c][0] = detect_corner_axis(pixels, width, height, c, 0, line);
        corner[c][1] = detect_corner_axis(pixels, width, height, c, 1, line);
        if (corner[c][0] < 0 || corner[c][1] < 0)
            return FALSE;
        radius[c] = MAX(corner[c][0], corner[c][1]);
        if (radius[c] < 2)
            return FALSE;
    }

    insets[0] = MAX(radius[0], radius[1]);
    insets[1] = MAX(radius[1], radius[2]);
    insets[2] = MAX(radius[2], radius[3]);
    insets[3] = MAX(radius[3], radius[0]);
    return TRUE;
}

gboolean meta_gnoblin_window_effects_detect_csd(ClutterActor* actor, double insets[4]) {
    CsdProbeCache* cache;
    MetaShapedTexture* texture;
    g_autoptr(GBytes) pixels_bytes = NULL;
    g_autoptr(GError) error = NULL;
    const guint8* pixels;
    gsize size;
    int width = 0;
    int height = 0;
    gboolean detected;
    double pixel_insets[4];
    guint attempts = 1;

    g_return_val_if_fail(CLUTTER_IS_ACTOR(actor), FALSE);
    g_return_val_if_fail(insets != NULL, FALSE);
    if (!META_IS_WINDOW_ACTOR(actor) || !clutter_actor_is_mapped(actor) ||
        clutter_actor_get_opacity(actor) != 255)
        return FALSE;

    texture = meta_window_actor_get_texture(META_WINDOW_ACTOR(actor));
    if (!texture)
        return FALSE;
    width = meta_shaped_texture_get_width(texture);
    height = meta_shaped_texture_get_height(texture);
    if (width < 16 || height < 16 || (gint64)width * height > CSD_MAX_PIXELS)
        return FALSE;
    if (clutter_actor_get_width(actor) <= 0 || clutter_actor_get_height(actor) <= 0)
        return FALSE;

    cache = g_object_get_data(G_OBJECT(actor), CSD_PROBE_CACHE_KEY);
    if (cache && cache->width == width && cache->height == height) {
        if (cache->detected) {
            csd_insets_to_actor_units(actor, width, height, cache->insets, insets);
            return TRUE;
        }
        if (cache->attempts >= 8)
            return FALSE;
        attempts = cache->attempts + 1;
    }

    pixels_bytes = meta_shaped_texture_get_pixels(texture, &width, &height, &error);
    if (!pixels_bytes || width < 16 || height < 16 || (gint64)width * height > CSD_MAX_PIXELS) {
        cache = g_new0(CsdProbeCache, 1);
        cache->width = meta_shaped_texture_get_width(texture);
        cache->height = meta_shaped_texture_get_height(texture);
        cache->attempts = attempts;
        g_object_set_data_full(G_OBJECT(actor), CSD_PROBE_CACHE_KEY, cache, g_free);
        return FALSE;
    }
    pixels = g_bytes_get_data(pixels_bytes, &size);
    if (size < (gsize)width * height * 4) {
        cache = g_new0(CsdProbeCache, 1);
        cache->width = width;
        cache->height = height;
        cache->attempts = attempts;
        g_object_set_data_full(G_OBJECT(actor), CSD_PROBE_CACHE_KEY, cache, g_free);
        return FALSE;
    }

    detected = detect_csd_insets(pixels, width, height, pixel_insets);
    cache = g_new0(CsdProbeCache, 1);
    cache->width = width;
    cache->height = height;
    cache->detected = detected;
    cache->attempts = attempts;
    if (detected)
        memcpy(cache->insets, pixel_insets, sizeof(cache->insets));
    g_object_set_data_full(G_OBJECT(actor), CSD_PROBE_CACHE_KEY, cache, g_free);
    if (detected)
        csd_insets_to_actor_units(actor, width, height, pixel_insets, insets);
    return detected;
}

typedef ClutterOffscreenEffectClass MetaGnoblinRoundedClipClass;

GType meta_gnoblin_rounded_clip_get_type(void);

#define META_TYPE_GNOBLIN_ROUNDED_CLIP (meta_gnoblin_rounded_clip_get_type())
#define META_GNOBLIN_ROUNDED_CLIP(value)                                                           \
    ((MetaGnoblinRoundedClip*)g_type_check_instance_cast((GTypeInstance*)(value),                  \
                                                         META_TYPE_GNOBLIN_ROUNDED_CLIP))
#define META_IS_GNOBLIN_ROUNDED_CLIP(value)                                                        \
    (g_type_check_instance_is_a((GTypeInstance*)(value), META_TYPE_GNOBLIN_ROUNDED_CLIP))

G_DEFINE_TYPE(MetaGnoblinRoundedClip, meta_gnoblin_rounded_clip, CLUTTER_TYPE_OFFSCREEN_EFFECT)

static CoglPipeline* rounded_clip_create_pipeline(ClutterOffscreenEffect* effect,
                                                  CoglTexture* texture) {
    CoglPipeline* pipeline = CLUTTER_OFFSCREEN_EFFECT_CLASS(meta_gnoblin_rounded_clip_parent_class)
                                 ->create_pipeline(effect, texture);
    CoglSnippet* snippet = cogl_snippet_new(
        COGL_SNIPPET_HOOK_FRAGMENT,
        "uniform vec4 gnoblin_rounded_clip_bounds;"
        "uniform float gnoblin_rounded_clip_radius;"
        "uniform float gnoblin_rounded_clip_exponent;"
        "uniform float gnoblin_rounded_clip_automatic;"
        "uniform float gnoblin_rounded_clip_csd;"
        "uniform vec4 gnoblin_rounded_clip_csd_insets;"
        "float sourceAlpha(vec2 point){"
        "vec2 uv=(point-gnoblin_rounded_clip_bounds.xy)/"
        "(gnoblin_rounded_clip_bounds.zw-gnoblin_rounded_clip_bounds.xy);"
        "return texture2D(cogl_sampler0,clamp(uv,vec2(0.0),vec2(1.0))).a;}"
        "vec4 sourcePixel(vec2 point){"
        "vec2 uv=(point-gnoblin_rounded_clip_bounds.xy)/"
        "(gnoblin_rounded_clip_bounds.zw-gnoblin_rounded_clip_bounds.xy);"
        "vec4 pixel=texture2D(cogl_sampler0,clamp(uv,vec2(0.0),vec2(1.0)));"
        "return vec4(pixel.rgb/max(pixel.a,0.00001),pixel.a);}"
        "vec4 flatPatch(vec2 point){"
        "vec4 a=sourcePixel(point+vec2(-1.0,-1.0));"
        "vec4 b=sourcePixel(point+vec2(1.0,-1.0));"
        "vec4 c=sourcePixel(point+vec2(-1.0,1.0));"
        "vec4 d=sourcePixel(point+vec2(1.0,1.0));"
        "vec4 spread=max(max(a,b),max(c,d))-min(min(a,b),min(c,d));"
        "if(max(max(spread.r,spread.g),spread.b)>0.08||spread.a>0.06)return vec4(0.0);"
        "return (a+b+c+d)*0.25;}"
        "bool sameBackground(vec4 a,vec4 b){"
        "vec4 delta=abs(a-b);"
        "return min(a.a,b.a)>0.2&&max(max(delta.r,delta.g),delta.b)<0.06&&delta.a<0.06;}"
        "vec4 cornerBackground(vec2 point,bool right,bool bottom){"
        "vec2 direction=vec2(right?-1.0:1.0,bottom?-1.0:1.0);"
        "vec2 origin=vec2(right?gnoblin_rounded_clip_bounds.z:gnoblin_rounded_clip_bounds.x,"
        "bottom?gnoblin_rounded_clip_bounds.w:gnoblin_rounded_clip_bounds.y);"
        "vec2 inset=vec2(right?gnoblin_rounded_clip_csd_insets.y:gnoblin_rounded_clip_csd_insets.w,"
        "bottom?gnoblin_rounded_clip_csd_insets.z:gnoblin_rounded_clip_csd_insets.x);"
        "vec2 pa=origin+direction*vec2(inset.x+4.0,4.0);"
        "vec2 pb=origin+direction*vec2(4.0,inset.y+4.0);"
        "vec2 pc=origin+direction*(inset+vec2(4.0));"
        "vec4 a=flatPatch(pa),b=flatPatch(pb),c=flatPatch(pc);"
        "if(!sameBackground(a,b)){"
        "if(sameBackground(a,c)){b=c;pb=pc;}"
        "else if(sameBackground(b,c)){a=c;pa=pc;}"
        "else return vec4(0.0);}"
        "float da=distance(point,pa),db=distance(point,pb);"
        "return mix(a,b,da/max(da+db,0.001));}"
        "bool squareCorner(vec2 point,float radius){"
        "vec2 middle=(gnoblin_rounded_clip_bounds.xy+"
        "gnoblin_rounded_clip_bounds.zw)*0.5;"
        "vec2 direction=vec2(point.x<middle.x?1.0:-1.0,"
        "point.y<middle.y?1.0:-1.0);"
        "vec2 origin=vec2(point.x<middle.x?gnoblin_rounded_clip_bounds.x:"
        "gnoblin_rounded_clip_bounds.z,point.y<middle.y?"
        "gnoblin_rounded_clip_bounds.y:gnoblin_rounded_clip_bounds.w);"
        "float inset=min(radius,8.0);"
        "float reference=sourceAlpha(origin+direction*inset);"
        "float diagonal=sourceAlpha(origin+direction*0.75);"
        "float horizontal=sourceAlpha(origin+direction*vec2(inset,0.75));"
        "float vertical=sourceAlpha(origin+direction*vec2(0.75,inset));"
        "return reference>0.02&&min(diagonal,min(horizontal,vertical))>=reference*0.85;}",
        "vec2 p=gnoblin_rounded_clip_bounds.xy+"
        "cogl_tex_coord_in[0].st*(gnoblin_rounded_clip_bounds.zw-"
        "gnoblin_rounded_clip_bounds.xy);"
        "vec2 half_size=(gnoblin_rounded_clip_bounds.zw-"
        "gnoblin_rounded_clip_bounds.xy)*0.5;"
        "float radius=min(gnoblin_rounded_clip_radius,min(half_size.x,half_size.y));"
        "if(radius>0.0){"
        "vec2 q=max(abs(p-(gnoblin_rounded_clip_bounds.xy+half_size))-"
        "(half_size-vec2(radius)),vec2(0.0));"
        "float distance=pow(pow(q.x/radius,gnoblin_rounded_clip_exponent)+"
        "pow(q.y/radius,gnoblin_rounded_clip_exponent),"
        "1.0/gnoblin_rounded_clip_exponent)*radius;"
        "bool corner=q.x>0.0&&q.y>0.0;"
        "bool right=p.x>(gnoblin_rounded_clip_bounds.x+"
        "gnoblin_rounded_clip_bounds.z)*0.5;"
        "bool bottom=p.y>(gnoblin_rounded_clip_bounds.y+"
        "gnoblin_rounded_clip_bounds.w)*0.5;"
        "float insetX=right?gnoblin_rounded_clip_csd_insets.y:"
        "gnoblin_rounded_clip_csd_insets.w;"
        "float insetY=bottom?gnoblin_rounded_clip_csd_insets.z:"
        "gnoblin_rounded_clip_csd_insets.x;"
        "float extendX=insetX+sqrt(2.0*insetX)+2.0;"
        "float extendY=insetY+sqrt(2.0*insetY)+2.0;"
        "bool csdCorner=gnoblin_rounded_clip_csd>0.5&&"
        "(right?gnoblin_rounded_clip_bounds.z-p.x:"
        "p.x-gnoblin_rounded_clip_bounds.x)<extendX&&"
        "(bottom?gnoblin_rounded_clip_bounds.w-p.y:"
        "p.y-gnoblin_rounded_clip_bounds.y)<extendY;"
        "if(csdCorner){"
        "vec4 fill=cornerBackground(p,right,bottom);"
        "vec2 dx=vec2(1.5,0.0),dy=vec2(0.0,1.5);"
        "float edgeAlpha=min(min(sourceAlpha(p-dx),sourceAlpha(p+dx)),"
        "min(sourceAlpha(p-dy),sourceAlpha(p+dy)));"
        "edgeAlpha=min(edgeAlpha,min(min(sourceAlpha(p-dx-dy),sourceAlpha(p+dx-dy)),"
        "min(sourceAlpha(p-dx+dy),sourceAlpha(p+dx+dy))));"
        "if(fill.a>0.2&&edgeAlpha<fill.a*0.98)"
        "cogl_color_out=vec4(fill.rgb*fill.a,fill.a)*cogl_color_in;}"
        "bool preserve=gnoblin_rounded_clip_automatic>0.5&&corner&&"
        "!squareCorner(p,radius)&&!csdCorner;"
        "float coverage=preserve?1.0:1.0-smoothstep(radius-0.5,radius+0.5,distance);"
        "cogl_color_out*=coverage;} ");
    cogl_pipeline_add_snippet(pipeline, snippet);
    g_object_unref(snippet);
    return pipeline;
}

static void rounded_clip_paint_target(ClutterOffscreenEffect* effect, ClutterPaintNode* node,
                                      ClutterPaintContext* paint_context) {
    MetaGnoblinRoundedClip* clip = META_GNOBLIN_ROUNDED_CLIP(effect);
    ClutterActor* actor = clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(effect));
    CoglPipeline* pipeline = clutter_offscreen_effect_get_pipeline(effect);
    graphene_rect_t target;
    float bounds[4];
    float scale_x;
    float scale_y;
    float radius;
    float padding_top;
    float padding_right;
    float padding_bottom;
    float padding_left;

    if (actor && clutter_offscreen_effect_get_target_rect(effect, &target)) {
        scale_x = target.size.width / MAX(clutter_actor_get_width(actor), 1.f);
        scale_y = target.size.height / MAX(clutter_actor_get_height(actor), 1.f);
        radius = (float)(clip->radius * MIN(scale_x, scale_y));
        padding_top = (float)(clip->padding[0] * scale_y);
        padding_right = (float)(clip->padding[1] * scale_x);
        padding_bottom = (float)(clip->padding[2] * scale_y);
        padding_left = (float)(clip->padding[3] * scale_x);
        bounds[0] = target.origin.x + padding_left;
        bounds[1] = target.origin.y + padding_top;
        bounds[2] = target.origin.x + target.size.width - padding_right;
        bounds[3] = target.origin.y + target.size.height - padding_bottom;
        if (bounds[2] <= bounds[0] || bounds[3] <= bounds[1])
            radius = 0.f;
        cogl_pipeline_set_uniform_float(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_rounded_clip_bounds"),
            4, 1, bounds);
        cogl_pipeline_set_uniform_1f(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_rounded_clip_radius"),
            radius);
        cogl_pipeline_set_uniform_1f(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_rounded_clip_exponent"),
            (float)clip->exponent);
        cogl_pipeline_set_uniform_1f(
            pipeline,
            cogl_pipeline_get_uniform_location(pipeline, "gnoblin_rounded_clip_automatic"),
            clip->automatic ? 1.f : 0.f);
        cogl_pipeline_set_uniform_1f(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_rounded_clip_csd"),
            clip->csd_reconstruction ? 1.f : 0.f);
        float csd_insets[4] = {
            (float)(clip->csd_insets[0] * scale_y),
            (float)(clip->csd_insets[1] * scale_x),
            (float)(clip->csd_insets[2] * scale_y),
            (float)(clip->csd_insets[3] * scale_x),
        };
        cogl_pipeline_set_uniform_float(
            pipeline,
            cogl_pipeline_get_uniform_location(pipeline, "gnoblin_rounded_clip_csd_insets"), 4, 1,
            csd_insets);
    } else {
        /* Uniforms persist on a pipeline. Force the fragment's no-op branch
         * while an actor is being detached or has no offscreen target. */
        cogl_pipeline_set_uniform_1f(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_rounded_clip_radius"),
            0.f);
    }
    CLUTTER_OFFSCREEN_EFFECT_CLASS(meta_gnoblin_rounded_clip_parent_class)
        ->paint_target(effect, node, paint_context);
}

static void meta_gnoblin_rounded_clip_class_init(MetaGnoblinRoundedClipClass* klass) {
    ClutterOffscreenEffectClass* effect_class = CLUTTER_OFFSCREEN_EFFECT_CLASS(klass);
    effect_class->create_pipeline = rounded_clip_create_pipeline;
    effect_class->paint_target = rounded_clip_paint_target;
}

static void meta_gnoblin_rounded_clip_init(MetaGnoblinRoundedClip* clip) {
    clip->exponent = 2;
}

void meta_gnoblin_window_effects_clear_rounded_clip(ClutterActor* actor) {
    g_return_if_fail(CLUTTER_IS_ACTOR(actor));

    ClutterEffect* effect = clutter_actor_get_effect(actor, ROUNDED_CLIP_EFFECT_NAME);
    if (!effect)
        return;
    if (!META_IS_GNOBLIN_ROUNDED_CLIP(effect)) {
        g_warning("Gnoblin rounded clip name is occupied by an unrelated effect");
        return;
    }
    clutter_actor_remove_effect(actor, effect);
}

void meta_gnoblin_window_effects_set_rounded_clip(ClutterActor* actor, double radius,
                                                  double exponent, gboolean automatic,
                                                  const double padding[4]) {
    MetaGnoblinRoundedClip* clip;
    ClutterEffect* effect;
    double clamped_padding[4];
    guint i;

    g_return_if_fail(CLUTTER_IS_ACTOR(actor));
    g_return_if_fail(padding != NULL);

    if (!isfinite(radius) || !isfinite(exponent) || radius <= 0) {
        meta_gnoblin_window_effects_clear_rounded_clip(actor);
        return;
    }

    for (i = 0; i < G_N_ELEMENTS(clamped_padding); i++) {
        if (!isfinite(padding[i])) {
            g_warning("Gnoblin rounded clip padding must contain only finite values");
            return;
        }
        clamped_padding[i] =
            CLAMP(padding[i], -ROUNDED_CLIP_PADDING_LIMIT, ROUNDED_CLIP_PADDING_LIMIT);
    }

    effect = clutter_actor_get_effect(actor, ROUNDED_CLIP_EFFECT_NAME);
    if (effect && !META_IS_GNOBLIN_ROUNDED_CLIP(effect)) {
        g_warning("Gnoblin rounded clip name is occupied by an unrelated effect");
        return;
    }
    radius = CLAMP(radius, 0., 4096.);
    exponent = CLAMP(exponent, 2., 6.);
    if (!effect) {
        clip = g_object_new(META_TYPE_GNOBLIN_ROUNDED_CLIP, NULL);
        clip->radius = radius;
        clip->exponent = exponent;
        clip->automatic = automatic;
        memcpy(clip->padding, clamped_padding, sizeof(clip->padding));
        clutter_actor_add_effect_with_name(actor, ROUNDED_CLIP_EFFECT_NAME, CLUTTER_EFFECT(clip));
        g_object_unref(clip);
        return;
    } else {
        clip = META_GNOBLIN_ROUNDED_CLIP(effect);
    }

    if (clip->radius == radius && clip->exponent == exponent && clip->automatic == automatic &&
        memcmp(clip->padding, clamped_padding, sizeof(clip->padding)) == 0)
        return;
    clip->radius = radius;
    clip->exponent = exponent;
    clip->automatic = automatic;
    memcpy(clip->padding, clamped_padding, sizeof(clip->padding));
    clutter_effect_queue_repaint(CLUTTER_EFFECT(clip));
}

void meta_gnoblin_window_effects_set_csd_reconstruction(ClutterActor* actor, gboolean enabled,
                                                        const double insets[4]) {
    ClutterEffect* effect;
    MetaGnoblinRoundedClip* clip;
    double zero_insets[4] = {0, 0, 0, 0};
    const double* values = insets ? insets : zero_insets;

    g_return_if_fail(CLUTTER_IS_ACTOR(actor));
    for (guint i = 0; i < 4; i++) {
        if (!isfinite(values[i]) || values[i] < 0) {
            g_warning("Gnoblin CSD insets must contain finite non-negative values");
            return;
        }
    }

    effect = clutter_actor_get_effect(actor, ROUNDED_CLIP_EFFECT_NAME);
    if (!effect || !META_IS_GNOBLIN_ROUNDED_CLIP(effect))
        return;
    clip = META_GNOBLIN_ROUNDED_CLIP(effect);
    if (clip->csd_reconstruction == enabled &&
        memcmp(clip->csd_insets, values, sizeof(clip->csd_insets)) == 0)
        return;
    clip->csd_reconstruction = enabled;
    memcpy(clip->csd_insets, values, sizeof(clip->csd_insets));
    clutter_effect_queue_repaint(CLUTTER_EFFECT(clip));
}
