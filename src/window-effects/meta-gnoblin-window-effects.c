/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Compositor-owned visual effects for managed client surface actors. */
#include "config.h"
#include "compositor/meta-gnoblin-window-effects.h"

#include <cogl/cogl.h>
#include <math.h>

#define ROUNDED_CLIP_EFFECT_NAME "gnoblin-rounded-clip"

typedef struct {
    ClutterOffscreenEffect parent;
    double radius;
    double exponent;
    gboolean automatic;
} MetaGnoblinRoundedClip;

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
        "float sourceAlpha(vec2 point){"
        "vec2 uv=(point-gnoblin_rounded_clip_bounds.xy)/"
        "(gnoblin_rounded_clip_bounds.zw-gnoblin_rounded_clip_bounds.xy);"
        "return texture2D(cogl_sampler0,clamp(uv,vec2(0.0),vec2(1.0))).a;}"
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
        "bool preserve=gnoblin_rounded_clip_automatic>0.5&&corner&&"
        "!squareCorner(p,radius);"
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

    if (actor && clutter_offscreen_effect_get_target_rect(effect, &target)) {
        scale_x = target.size.width / MAX(clutter_actor_get_width(actor), 1.f);
        scale_y = target.size.height / MAX(clutter_actor_get_height(actor), 1.f);
        radius = (float)(clip->radius * MIN(scale_x, scale_y));
        bounds[0] = target.origin.x;
        bounds[1] = target.origin.y;
        bounds[2] = target.origin.x + target.size.width;
        bounds[3] = target.origin.y + target.size.height;
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
                                                  double exponent, gboolean automatic) {
    MetaGnoblinRoundedClip* clip;
    ClutterEffect* effect;

    g_return_if_fail(CLUTTER_IS_ACTOR(actor));

    if (!isfinite(radius) || !isfinite(exponent) || radius <= 0) {
        meta_gnoblin_window_effects_clear_rounded_clip(actor);
        return;
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
        clutter_actor_add_effect_with_name(actor, ROUNDED_CLIP_EFFECT_NAME, CLUTTER_EFFECT(clip));
        g_object_unref(clip);
        return;
    } else {
        clip = META_GNOBLIN_ROUNDED_CLIP(effect);
    }

    if (clip->radius == radius && clip->exponent == exponent && clip->automatic == automatic)
        return;
    clip->radius = radius;
    clip->exponent = exponent;
    clip->automatic = automatic;
    clutter_effect_queue_repaint(CLUTTER_EFFECT(clip));
}
