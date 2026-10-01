/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Compositor-owned visual effects for managed client surface actors. */
#include "config.h"
#include "compositor/meta-gnoblin-window-effects.h"

#include <clutter/clutter.h>
#include <cogl/cogl.h>
#include <math.h>
#include <string.h>

#include "meta/meta-shaped-texture.h"
#include "meta/meta-window-actor.h"
#include "meta/prefs.h"

#define ROUNDED_CLIP_EFFECT_NAME "gnoblin-rounded-clip"
#define ROUNDED_CLIP_PADDING_LIMIT 128.
#define CSD_PROBE_CACHE_KEY "gnoblin-csd-probe-cache"
#define CSD_MAX_PIXELS 16000000
#define WINDOW_SHADOW_ACTOR_KEY "gnoblin-window-shadow"
#define WINDOW_SHADOW_EFFECT_NAME "gnoblin-window-shadow-effect"

int meta_shaped_texture_get_width(MetaShapedTexture* texture);
int meta_shaped_texture_get_height(MetaShapedTexture* texture);

typedef struct {
    ClutterOffscreenEffect parent;
    double bounds[4];
    double radius;
    double exponent;
    double from_geometry[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4];
    double from_color[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4];
    double to_geometry[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4];
    double to_color[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4];
    guint n_from;
    guint n_to;
    double progress;
    guint current_padding;
    guint target_padding;
    ClutterTimeline* timeline;
    guint duration_ms;
    char* easing;
    gboolean has_bezier;
    double bezier[4];
    gboolean queued;
    double queued_geometry[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4];
    double queued_color[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4];
    guint queued_count;
    guint queued_duration_ms;
    char* queued_easing;
    gboolean queued_has_bezier;
    double queued_bezier[4];
    double actor_bounds[4];
} MetaGnoblinWindowShadowEffect;

typedef struct {
    ClutterOffscreenEffectClass parent_class;
} MetaGnoblinWindowShadowEffectClass;

#define META_TYPE_GNOBLIN_WINDOW_SHADOW_EFFECT (meta_gnoblin_window_shadow_effect_get_type())
#define META_GNOBLIN_WINDOW_SHADOW_EFFECT(value)                                                   \
    ((MetaGnoblinWindowShadowEffect*)g_type_check_instance_cast(                                   \
        (GTypeInstance*)(value), META_TYPE_GNOBLIN_WINDOW_SHADOW_EFFECT))
#define META_IS_GNOBLIN_WINDOW_SHADOW_EFFECT(value)                                                \
    (g_type_check_instance_is_a((GTypeInstance*)(value), META_TYPE_GNOBLIN_WINDOW_SHADOW_EFFECT))

GType meta_gnoblin_window_shadow_effect_get_type(void);
G_DEFINE_TYPE(MetaGnoblinWindowShadowEffect, meta_gnoblin_window_shadow_effect,
              CLUTTER_TYPE_OFFSCREEN_EFFECT)

static double shadow_ease_progress(const MetaGnoblinWindowShadowEffect* effect, double progress) {
    const double t = CLAMP(progress, 0., 1.);
    if (effect->has_bezier) {
        const double x1 = effect->bezier[0], y1 = effect->bezier[1];
        const double x2 = effect->bezier[2], y2 = effect->bezier[3];
        double low = 0., high = 1., u = t;
        for (guint i = 0; i < 16; i++) {
            const double one_minus_u = 1. - u;
            const double x =
                3. * one_minus_u * one_minus_u * u * x1 + 3. * one_minus_u * u * u * x2 + u * u * u;
            if (fabs(x - t) < 0.0001)
                break;
            if (x < t)
                low = u;
            else
                high = u;
            u = (low + high) * 0.5;
        }
        const double one_minus_u = 1. - u;
        return 3. * one_minus_u * one_minus_u * u * y1 + 3. * one_minus_u * u * u * y2 + u * u * u;
    }

    if (g_strcmp0(effect->easing, "ease-in-quad") == 0)
        return t * t;
    if (g_strcmp0(effect->easing, "ease-out-quad") == 0)
        return 1. - (1. - t) * (1. - t);
    if (g_strcmp0(effect->easing, "ease-in-cubic") == 0)
        return t * t * t;
    if (g_strcmp0(effect->easing, "ease-in-out-cubic") == 0)
        return t < 0.5 ? 4. * t * t * t : 1. - pow(-2. * t + 2., 3.) / 2.;
    if (g_strcmp0(effect->easing, "ease-out-expo") == 0)
        return t >= 1. ? 1. : 1. - pow(2., -10. * t);
    if (g_strcmp0(effect->easing, "ease-out-back") == 0) {
        const double c1 = 1.70158, c3 = c1 + 1.;
        return 1. + c3 * pow(t - 1., 3.) + c1 * pow(t - 1., 2.);
    }
    if (g_strcmp0(effect->easing, "ease-out-cubic") == 0)
        return 1. - pow(1. - t, 3.);
    return t;
}

static CoglPipeline* window_shadow_create_pipeline(ClutterOffscreenEffect* effect,
                                                   CoglTexture* texture) {
    CoglPipeline* pipeline =
        CLUTTER_OFFSCREEN_EFFECT_CLASS(meta_gnoblin_window_shadow_effect_parent_class)
            ->create_pipeline(effect, texture);
    GString* declarations = g_string_new("uniform vec4 gnoblin_shadow_texture_bounds;"
                                         "uniform vec4 gnoblin_shadow_bounds;"
                                         "uniform float gnoblin_shadow_radius;"
                                         "uniform float gnoblin_shadow_exponent;"
                                         "uniform float gnoblin_shadow_progress;");
    GString* code =
        g_string_new("vec2 point=gnoblin_shadow_texture_bounds.xy+cogl_tex_coord_in[0].st*"
                     "(gnoblin_shadow_texture_bounds.zw-gnoblin_shadow_texture_bounds.xy);");

    for (guint i = 0; i < META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS; i++) {
        g_string_append_printf(declarations,
                               "uniform vec4 gnoblin_shadow_geometry_a%u;"
                               "uniform vec4 gnoblin_shadow_color_a%u;"
                               "uniform vec4 gnoblin_shadow_geometry_b%u;"
                               "uniform vec4 gnoblin_shadow_color_b%u;",
                               i, i, i, i);
    }
    g_string_append(code,
                    "vec4 resultA=vec4(0.0),resultB=vec4(0.0);"
                    "vec2 size=gnoblin_shadow_bounds.zw-gnoblin_shadow_bounds.xy;"
                    "vec2 halfSize=size*0.5;"
                    "float radius=min(gnoblin_shadow_radius,min(halfSize.x,halfSize.y));"
                    "vec2 d=abs(point-(gnoblin_shadow_bounds.xy+halfSize))-halfSize+vec2(radius);"
                    "vec2 q=max(d,vec2(0.0));"
                    "float longest=max(q.x,q.y);"
                    "float cornerDistance=longest<0.001?0.0:longest*"
                    "pow(pow(q.x/longest,gnoblin_shadow_exponent)+"
                    "pow(q.y/longest,gnoblin_shadow_exponent),1.0/gnoblin_shadow_exponent);"
                    "float shapeDistance=min(max(d.x,d.y),0.0)+cornerDistance-radius;"
                    "float clipCoverage=clamp(0.5-shapeDistance,0.0,1.0);");

    for (guint i = 0; i < META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS; i++) {
        g_string_append_printf(
            code,
            "if(gnoblin_shadow_color_a%u.a>0.0){"
            "vec4 g=gnoblin_shadow_geometry_a%u,c=gnoblin_shadow_color_a%u;"
            "vec2 hs=halfSize+vec2(g.z);if(min(hs.x,hs.y)>0.0){"
            "float layerRadius=min(max(0.0,gnoblin_shadow_radius+g.z),min(hs.x,hs.y));"
            "vec2 ld=abs((point-g.xy)-(gnoblin_shadow_bounds.xy+halfSize))-hs+vec2(layerRadius);"
            "vec2 lq=max(ld,vec2(0.0));float ll=max(lq.x,lq.y);"
            "float cd=ll<0.001?0.0:ll*pow(pow(lq.x/ll,gnoblin_shadow_exponent)+"
            "pow(lq.y/ll,gnoblin_shadow_exponent),1.0/gnoblin_shadow_exponent);"
            "float dist=min(max(ld.x,ld.y),0.0)+cd-layerRadius;"
            "float density=g.w<0.01?clamp(0.5-dist,0.0,1.0):"
            "1.0/(1.0+exp(clamp(3.4*dist/g.w,-30.0,30.0)));"
            "float alpha=c.a*density*(1.0-clipCoverage);"
            "vec4 layer=vec4(c.rgb*alpha,alpha);resultA=layer+resultA*(1.0-layer.a);}}"
            "if(gnoblin_shadow_color_b%u.a>0.0){"
            "vec4 g=gnoblin_shadow_geometry_b%u,c=gnoblin_shadow_color_b%u;"
            "vec2 hs=halfSize+vec2(g.z);if(min(hs.x,hs.y)>0.0){"
            "float layerRadius=min(max(0.0,gnoblin_shadow_radius+g.z),min(hs.x,hs.y));"
            "vec2 ld=abs((point-g.xy)-(gnoblin_shadow_bounds.xy+halfSize))-hs+vec2(layerRadius);"
            "vec2 lq=max(ld,vec2(0.0));float ll=max(lq.x,lq.y);"
            "float cd=ll<0.001?0.0:ll*pow(pow(lq.x/ll,gnoblin_shadow_exponent)+"
            "pow(lq.y/ll,gnoblin_shadow_exponent),1.0/gnoblin_shadow_exponent);"
            "float dist=min(max(ld.x,ld.y),0.0)+cd-layerRadius;"
            "float density=g.w<0.01?clamp(0.5-dist,0.0,1.0):"
            "1.0/(1.0+exp(clamp(3.4*dist/g.w,-30.0,30.0)));"
            "float alpha=c.a*density*(1.0-clipCoverage);"
            "vec4 layer=vec4(c.rgb*alpha,alpha);resultB=layer+resultB*(1.0-layer.a);}}",
            i, i, i, i, i, i);
    }
    g_string_append(code,
                    "cogl_color_out=mix(resultA,resultB,gnoblin_shadow_progress)*cogl_color_in;");

    CoglSnippet* snippet =
        cogl_snippet_new(COGL_SNIPPET_HOOK_FRAGMENT, declarations->str, code->str);
    cogl_pipeline_add_snippet(pipeline, snippet);
    g_object_unref(snippet);
    g_string_free(declarations, TRUE);
    g_string_free(code, TRUE);
    return pipeline;
}

static void window_shadow_set_uniform_layers(
    CoglPipeline* pipeline, const char* geometry_prefix, const char* color_prefix,
    double geometry[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4],
    double color[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4], float scale_x, float scale_y) {
    const float scale = MIN(scale_x, scale_y);
    for (guint i = 0; i < META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS; i++) {
        g_autofree char* geometry_name =
            g_strdup_printf("gnoblin_shadow_geometry_%s%u", geometry_prefix, i);
        g_autofree char* color_name = g_strdup_printf("gnoblin_shadow_color_%s%u", color_prefix, i);
        const float geometry_values[4] = {
            (float)(geometry[i][0] * scale_x), (float)(geometry[i][1] * scale_y),
            (float)(geometry[i][2] * scale), (float)(geometry[i][3] * scale)};
        const float color_values[4] = {(float)color[i][0], (float)color[i][1], (float)color[i][2],
                                       (float)color[i][3]};
        cogl_pipeline_set_uniform_float(pipeline,
                                        cogl_pipeline_get_uniform_location(pipeline, geometry_name),
                                        4, 1, geometry_values);
        cogl_pipeline_set_uniform_float(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, color_name), 4, 1, color_values);
    }
}

static void window_shadow_paint_target(ClutterOffscreenEffect* effect, ClutterPaintNode* node,
                                       ClutterPaintContext* paint_context) {
    MetaGnoblinWindowShadowEffect* shadow = META_GNOBLIN_WINDOW_SHADOW_EFFECT(effect);
    ClutterActor* actor = clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(effect));
    CoglPipeline* pipeline = clutter_offscreen_effect_get_pipeline(effect);
    graphene_rect_t target;
    float bounds[4];
    float texture_bounds[4];
    float scale_x, scale_y;

    if (actor && clutter_offscreen_effect_get_target_rect(effect, &target)) {
        scale_x = target.size.width / MAX(clutter_actor_get_width(actor), 1.f);
        scale_y = target.size.height / MAX(clutter_actor_get_height(actor), 1.f);
        bounds[0] = target.origin.x + (float)shadow->bounds[0] * scale_x;
        bounds[1] = target.origin.y + (float)shadow->bounds[1] * scale_y;
        bounds[2] = target.origin.x + (float)shadow->bounds[2] * scale_x;
        bounds[3] = target.origin.y + (float)shadow->bounds[3] * scale_y;
        texture_bounds[0] = target.origin.x;
        texture_bounds[1] = target.origin.y;
        texture_bounds[2] = target.origin.x + target.size.width;
        texture_bounds[3] = target.origin.y + target.size.height;
        cogl_pipeline_set_uniform_float(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_shadow_texture_bounds"),
            4, 1, texture_bounds);
        cogl_pipeline_set_uniform_float(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_shadow_bounds"), 4, 1,
            bounds);
        cogl_pipeline_set_uniform_1f(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_shadow_radius"),
            (float)(shadow->radius * MIN(scale_x, scale_y)));
        cogl_pipeline_set_uniform_1f(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_shadow_exponent"),
            (float)shadow->exponent);
        cogl_pipeline_set_uniform_1f(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_shadow_progress"),
            (float)shadow->progress);
        window_shadow_set_uniform_layers(pipeline, "a", "a", shadow->from_geometry,
                                         shadow->from_color, scale_x, scale_y);
        window_shadow_set_uniform_layers(pipeline, "b", "b", shadow->to_geometry, shadow->to_color,
                                         scale_x, scale_y);
    } else {
        cogl_pipeline_set_uniform_1f(
            pipeline, cogl_pipeline_get_uniform_location(pipeline, "gnoblin_shadow_progress"), 0.f);
    }
    CLUTTER_OFFSCREEN_EFFECT_CLASS(meta_gnoblin_window_shadow_effect_parent_class)
        ->paint_target(effect, node, paint_context);
}

static void window_shadow_effect_dispose(GObject* object) {
    MetaGnoblinWindowShadowEffect* shadow = META_GNOBLIN_WINDOW_SHADOW_EFFECT(object);
    if (shadow->timeline) {
        g_signal_handlers_disconnect_by_data(shadow->timeline, shadow);
        clutter_timeline_stop(shadow->timeline);
        g_clear_object(&shadow->timeline);
    }
    g_clear_pointer(&shadow->easing, g_free);
    g_clear_pointer(&shadow->queued_easing, g_free);
    G_OBJECT_CLASS(meta_gnoblin_window_shadow_effect_parent_class)->dispose(object);
}

static void
meta_gnoblin_window_shadow_effect_class_init(MetaGnoblinWindowShadowEffectClass* klass) {
    GObjectClass* object_class = G_OBJECT_CLASS(klass);
    ClutterOffscreenEffectClass* effect_class = CLUTTER_OFFSCREEN_EFFECT_CLASS(klass);
    object_class->dispose = window_shadow_effect_dispose;
    effect_class->create_pipeline = window_shadow_create_pipeline;
    effect_class->paint_target = window_shadow_paint_target;
}

static void meta_gnoblin_window_shadow_effect_init(MetaGnoblinWindowShadowEffect* shadow) {
    shadow->exponent = 2;
    shadow->progress = 1;
    shadow->easing = g_strdup("linear");
}

static void window_shadow_layout(ClutterActor* parent, ClutterActor* actor,
                                 MetaGnoblinWindowShadowEffect* effect, const double bounds[4],
                                 guint padding) {
    const double width = MAX(0., bounds[2] - bounds[0]);
    const double height = MAX(0., bounds[3] - bounds[1]);
    effect->bounds[0] = padding;
    effect->bounds[1] = padding;
    effect->bounds[2] = padding + width;
    effect->bounds[3] = padding + height;
    memcpy(effect->actor_bounds, bounds, sizeof(effect->actor_bounds));
    clutter_actor_set_position(actor, bounds[0] - padding, bounds[1] - padding);
    clutter_actor_set_size(actor, width + padding * 2., height + padding * 2.);
    clutter_actor_queue_relayout(parent);
    clutter_actor_queue_redraw(actor);
    clutter_actor_invalidate_paint_volume(parent);
}

static guint window_shadow_padding(const double geometry[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4],
                                   guint n_layers) {
    double padding = 2.;
    for (guint i = 0; i < n_layers; i++) {
        const double* layer = geometry[i];
        padding =
            MAX(padding, layer[3] * 2. + fabs(layer[2]) + MAX(fabs(layer[0]), fabs(layer[1])) + 2.);
    }
    return (guint)ceil(padding);
}

static void window_shadow_stop_timeline(MetaGnoblinWindowShadowEffect* effect) {
    if (!effect->timeline)
        return;
    g_signal_handlers_disconnect_by_data(effect->timeline, effect);
    clutter_timeline_stop(effect->timeline);
    g_clear_object(&effect->timeline);
}

static void
window_shadow_start_transition(MetaGnoblinWindowShadowEffect* effect,
                               const double geometry[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4],
                               const double color[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4],
                               guint n_layers, const MetaGnoblinWindowShadowTransition* transition);

static void window_shadow_timeline_new_frame(ClutterTimeline* timeline, gint elapsed,
                                             gpointer user_data) {
    MetaGnoblinWindowShadowEffect* shadow = user_data;
    shadow->progress = shadow_ease_progress(shadow, elapsed / (double)shadow->duration_ms);
    clutter_effect_queue_repaint(CLUTTER_EFFECT(shadow));
}

static void window_shadow_timeline_completed(ClutterTimeline* timeline, gpointer user_data) {
    MetaGnoblinWindowShadowEffect* shadow = user_data;
    shadow->progress = 1.;
    memcpy(shadow->from_geometry, shadow->to_geometry, sizeof(shadow->from_geometry));
    memcpy(shadow->from_color, shadow->to_color, sizeof(shadow->from_color));
    shadow->n_from = shadow->n_to;
    window_shadow_stop_timeline(shadow);
    ClutterActor* shadow_actor = clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(shadow));
    ClutterActor* parent = shadow_actor ? clutter_actor_get_parent(shadow_actor) : NULL;
    if (shadow->queued) {
        shadow->target_padding =
            window_shadow_padding(shadow->queued_geometry, shadow->queued_count);
        shadow->current_padding =
            MAX(window_shadow_padding(shadow->to_geometry, shadow->n_to), shadow->target_padding);
        if (parent)
            window_shadow_layout(parent, shadow_actor, shadow, shadow->actor_bounds,
                                 shadow->current_padding);
        MetaGnoblinWindowShadowTransition transition = {
            .duration_ms = shadow->queued_duration_ms,
            .easing = shadow->queued_easing,
            .has_bezier = shadow->queued_has_bezier,
        };
        memcpy(transition.bezier, shadow->queued_bezier, sizeof(transition.bezier));
        shadow->queued = FALSE;
        window_shadow_start_transition(shadow, shadow->queued_geometry, shadow->queued_color,
                                       shadow->queued_count, &transition);
    } else {
        shadow->current_padding = shadow->target_padding;
        if (parent)
            window_shadow_layout(parent, shadow_actor, shadow, shadow->actor_bounds,
                                 shadow->target_padding);
    }
    clutter_effect_queue_repaint(CLUTTER_EFFECT(shadow));
}

static void
window_shadow_start_transition(MetaGnoblinWindowShadowEffect* effect,
                               const double geometry[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4],
                               const double color[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4],
                               guint n_layers,
                               const MetaGnoblinWindowShadowTransition* transition) {
    ClutterActor* actor = clutter_actor_meta_get_actor(CLUTTER_ACTOR_META(effect));
    memcpy(effect->from_geometry, effect->to_geometry, sizeof(effect->from_geometry));
    memcpy(effect->from_color, effect->to_color, sizeof(effect->from_color));
    effect->n_from = effect->n_to;
    memcpy(effect->to_geometry, geometry, sizeof(effect->to_geometry));
    memcpy(effect->to_color, color, sizeof(effect->to_color));
    effect->n_to = n_layers;
    effect->progress = 0.;
    effect->duration_ms = transition ? MIN(transition->duration_ms, 2000u) : 0;
    g_free(effect->easing);
    effect->easing =
        g_strdup(transition && transition->easing ? transition->easing : "ease-out-cubic");
    effect->has_bezier = transition && transition->has_bezier;
    if (effect->has_bezier)
        memcpy(effect->bezier, transition->bezier, sizeof(effect->bezier));

    if (!effect->duration_ms || !meta_prefs_get_gnome_animations()) {
        effect->progress = 1.;
        memcpy(effect->from_geometry, effect->to_geometry, sizeof(effect->from_geometry));
        memcpy(effect->from_color, effect->to_color, sizeof(effect->from_color));
        effect->n_from = effect->n_to;
        return;
    }

    effect->timeline = clutter_timeline_new_for_actor(actor, effect->duration_ms);
    clutter_timeline_set_progress_mode(effect->timeline, CLUTTER_LINEAR);
    g_signal_connect(effect->timeline, "new-frame", G_CALLBACK(window_shadow_timeline_new_frame),
                     effect);
    g_signal_connect(effect->timeline, "completed", G_CALLBACK(window_shadow_timeline_completed),
                     effect);
    clutter_timeline_start(effect->timeline);
}

static gboolean
window_shadow_layers_equal(MetaGnoblinWindowShadowEffect* effect,
                           const double geometry[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4],
                           const double color[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4],
                           guint n_layers) {
    if (effect->n_to != n_layers)
        return FALSE;
    return memcmp(effect->to_geometry, geometry, sizeof(effect->to_geometry)) == 0 &&
           memcmp(effect->to_color, color, sizeof(effect->to_color)) == 0;
}

static void window_shadow_destroy(ClutterActor* window_actor) {
    ClutterActor* shadow_actor = g_object_get_data(G_OBJECT(window_actor), WINDOW_SHADOW_ACTOR_KEY);
    if (!shadow_actor)
        return;
    g_object_set_data(G_OBJECT(window_actor), WINDOW_SHADOW_ACTOR_KEY, NULL);
    clutter_actor_destroy(shadow_actor);
}

gboolean meta_gnoblin_window_effects_has_window_shadow(ClutterActor* window_actor) {
    g_return_val_if_fail(CLUTTER_IS_ACTOR(window_actor), FALSE);
    ClutterActor* shadow_actor = g_object_get_data(G_OBJECT(window_actor), WINDOW_SHADOW_ACTOR_KEY);
    if (!shadow_actor)
        return FALSE;
    ClutterEffect* attached = clutter_actor_get_effect(shadow_actor, WINDOW_SHADOW_EFFECT_NAME);
    if (!attached || !META_IS_GNOBLIN_WINDOW_SHADOW_EFFECT(attached))
        return FALSE;
    MetaGnoblinWindowShadowEffect* effect = META_GNOBLIN_WINDOW_SHADOW_EFFECT(attached);
    return effect->n_to > 0 || (effect->progress < 1. && effect->n_from > 0);
}

void meta_gnoblin_window_effects_set_window_shadow(
    ClutterActor* window_actor, gboolean enabled, const double bounds[4], double radius,
    double exponent, const MetaGnoblinWindowShadowLayer* layers, guint n_layers,
    const MetaGnoblinWindowShadowTransition* transition, guint child_index) {
    ClutterActor* shadow_actor;
    MetaGnoblinWindowShadowEffect* effect;
    double layer_geometry[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4] = {{0}};
    double layer_color[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS][4] = {{0}};

    g_return_if_fail(CLUTTER_IS_ACTOR(window_actor));
    if (!enabled || !bounds || !layers || n_layers == 0 ||
        n_layers > META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS) {
        window_shadow_destroy(window_actor);
        return;
    }
    for (guint i = 0; i < 4; i++) {
        if (!isfinite(bounds[i]))
            return;
    }
    if (!isfinite(radius) || !isfinite(exponent) || bounds[2] <= bounds[0] ||
        bounds[3] <= bounds[1])
        return;

    for (guint i = 0; i < n_layers; i++) {
        const MetaGnoblinWindowShadowLayer* layer = &layers[i];
        if (!isfinite(layer->x) || !isfinite(layer->y) || !isfinite(layer->blur) ||
            !isfinite(layer->spread) || !isfinite(layer->opacity))
            return;
        layer_geometry[i][0] = CLAMP(layer->x, -100., 100.);
        layer_geometry[i][1] = CLAMP(layer->y, -100., 100.);
        layer_geometry[i][2] = CLAMP(layer->spread, -100., 100.);
        layer_geometry[i][3] = CLAMP(layer->blur, 0., 100.);
        for (guint j = 0; j < 4; j++) {
            if (!isfinite(layer->color[j]))
                return;
            layer_color[i][j] = CLAMP(layer->color[j], 0., 1.);
        }
        layer_color[i][3] *= CLAMP(layer->opacity, 0., 1.);
    }

    shadow_actor = g_object_get_data(G_OBJECT(window_actor), WINDOW_SHADOW_ACTOR_KEY);
    if (!shadow_actor) {
        shadow_actor = clutter_actor_new();
        clutter_actor_set_reactive(shadow_actor, FALSE);
        clutter_actor_set_background_color(shadow_actor, &COGL_COLOR_INIT(255, 255, 255, 255));
        effect = g_object_new(META_TYPE_GNOBLIN_WINDOW_SHADOW_EFFECT, NULL);
        clutter_actor_add_effect_with_name(shadow_actor, WINDOW_SHADOW_EFFECT_NAME,
                                           CLUTTER_EFFECT(effect));
        g_object_unref(effect);
        const guint n_children = clutter_actor_get_n_children(window_actor);
        clutter_actor_insert_child_at_index(window_actor, shadow_actor,
                                            MIN(child_index, n_children));
        g_object_set_data(G_OBJECT(window_actor), WINDOW_SHADOW_ACTOR_KEY, shadow_actor);
        clutter_actor_show(shadow_actor);
    }
    const guint n_children = clutter_actor_get_n_children(window_actor);
    if (n_children > 0)
        clutter_actor_set_child_at_index(window_actor, shadow_actor,
                                         MIN(child_index, n_children - 1));
    ClutterEffect* attached = clutter_actor_get_effect(shadow_actor, WINDOW_SHADOW_EFFECT_NAME);
    effect = attached && META_IS_GNOBLIN_WINDOW_SHADOW_EFFECT(attached)
                 ? META_GNOBLIN_WINDOW_SHADOW_EFFECT(attached)
                 : NULL;
    if (!effect)
        return;

    effect->radius = CLAMP(radius, 0., 4096.);
    effect->exponent = CLAMP(exponent, 2., 6.);
    effect->target_padding = window_shadow_padding(layer_geometry, n_layers);
    guint padding = effect->timeline ? MAX(effect->current_padding, effect->target_padding)
                                     : effect->target_padding;
    effect->current_padding = padding;
    window_shadow_layout(window_actor, shadow_actor, effect, bounds, padding);

    if (!window_shadow_layers_equal(effect, layer_geometry, layer_color, n_layers)) {
        if (effect->timeline && transition && transition->duration_ms > 0 &&
            meta_prefs_get_gnome_animations()) {
            effect->queued = TRUE;
            memcpy(effect->queued_geometry, layer_geometry, sizeof(effect->queued_geometry));
            memcpy(effect->queued_color, layer_color, sizeof(effect->queued_color));
            effect->queued_count = n_layers;
            effect->queued_duration_ms = MIN(transition->duration_ms, 2000u);
            g_free(effect->queued_easing);
            effect->queued_easing =
                g_strdup(transition->easing ? transition->easing : "ease-out-cubic");
            effect->queued_has_bezier = transition->has_bezier;
            if (transition->has_bezier)
                memcpy(effect->queued_bezier, transition->bezier, sizeof(effect->queued_bezier));
        } else {
            window_shadow_stop_timeline(effect);
            effect->queued = FALSE;
            window_shadow_start_transition(effect, layer_geometry, layer_color, n_layers,
                                           transition);
        }
    } else {
        effect->queued = FALSE;
    }
    clutter_effect_queue_repaint(CLUTTER_EFFECT(effect));
}

typedef struct {
    ClutterOffscreenEffect parent;
    double radius;
    double exponent;
    gboolean automatic;
    double padding[4];
    double border_width;
    double border_color[4];
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
        "uniform float gnoblin_rounded_clip_border_width;"
        "uniform vec4 gnoblin_rounded_clip_border_color;"
        "float sourceAlpha(vec2 point){"
        "vec2 uv=(point-gnoblin_rounded_clip_bounds.xy)/"
        "(gnoblin_rounded_clip_bounds.zw-gnoblin_rounded_clip_bounds.xy);"
        "return texture2D(cogl_sampler0,clamp(uv,vec2(0.0),vec2(1.0))).a;}"
        "vec4 sourcePixel(vec2 point){"
        "vec2 uv=(point-gnoblin_rounded_clip_bounds.xy)/"
        "(gnoblin_rounded_clip_bounds.zw-gnoblin_rounded_clip_bounds.xy);"
        "vec4 pixel=texture2D(cogl_sampler0,clamp(uv,vec2(0.0),vec2(1.0)));"
        "return vec4(pixel.rgb/max(pixel.a,0.00001),pixel.a);}"
        "float roundedCoverage(vec2 point,vec4 box,float radius,float exponent){"
        "if(point.x<box.x||point.y<box.y||point.x>box.z||point.y>box.w)return 0.0;"
        "vec2 halfSize=(box.zw-box.xy)*0.5;"
        "radius=min(radius,min(halfSize.x,halfSize.y));"
        "if(radius<0.5)return 1.0;"
        "vec2 q=max(abs(point-(box.xy+halfSize))-(halfSize-vec2(radius)),vec2(0.0));"
        "float distance=pow(pow(q.x/max(radius,0.001),exponent)+"
        "pow(q.y/max(radius,0.001),exponent),1.0/exponent)*radius;"
        "return 1.0-smoothstep(radius-0.5,radius+0.5,distance);}"
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
        "float borderWidth=gnoblin_rounded_clip_border_width;"
        "if(abs(borderWidth)>0.01){"
        "float borderCoverage;"
        "if(borderWidth>0.0){"
        "vec4 innerBounds=gnoblin_rounded_clip_bounds+"
        "vec4(borderWidth,borderWidth,-borderWidth,-borderWidth);"
        "float innerRadius=max(0.0,radius-borderWidth);"
        "borderCoverage=max(0.0,coverage-roundedCoverage(p,innerBounds,innerRadius,"
        "gnoblin_rounded_clip_exponent));}"
        "else{"
        "float outsideWidth=-borderWidth;"
        "vec4 outerBounds=gnoblin_rounded_clip_bounds+"
        "vec4(-outsideWidth,-outsideWidth,outsideWidth,outsideWidth);"
        "float outerCoverage=roundedCoverage(p,outerBounds,radius+outsideWidth,"
        "gnoblin_rounded_clip_exponent);"
        "borderCoverage=max(0.0,outerCoverage-coverage);}"
        "float borderAlpha=borderCoverage*gnoblin_rounded_clip_border_color.a;"
        "vec4 border=vec4(gnoblin_rounded_clip_border_color.rgb*borderAlpha,borderAlpha);"
        "cogl_color_out=border*cogl_color_in+"
        "cogl_color_out*coverage*(1.0-borderAlpha);}"
        "else cogl_color_out*=coverage;} ");
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
    float border_width;
    float border_color[4];
    float padding_top;
    float padding_right;
    float padding_bottom;
    float padding_left;

    if (actor && clutter_offscreen_effect_get_target_rect(effect, &target)) {
        scale_x = target.size.width / MAX(clutter_actor_get_width(actor), 1.f);
        scale_y = target.size.height / MAX(clutter_actor_get_height(actor), 1.f);
        radius = (float)(clip->radius * MIN(scale_x, scale_y));
        border_width = (float)(clip->border_width * MIN(scale_x, scale_y));
        for (guint i = 0; i < G_N_ELEMENTS(border_color); i++)
            border_color[i] = (float)clip->border_color[i];
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
        cogl_pipeline_set_uniform_1f(
            pipeline,
            cogl_pipeline_get_uniform_location(pipeline, "gnoblin_rounded_clip_border_width"),
            border_width);
        cogl_pipeline_set_uniform_float(
            pipeline,
            cogl_pipeline_get_uniform_location(pipeline, "gnoblin_rounded_clip_border_color"), 4, 1,
            border_color);
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
    clip->border_color[0] = 128. / 255.;
    clip->border_color[1] = 128. / 255.;
    clip->border_color[2] = 128. / 255.;
    clip->border_color[3] = 1.;
}

static MetaGnoblinRoundedClip* find_rounded_clip(ClutterActor* actor) {
    ClutterEffect* effect = clutter_actor_get_effect(actor, ROUNDED_CLIP_EFFECT_NAME);
    if (effect && META_IS_GNOBLIN_ROUNDED_CLIP(effect))
        return META_GNOBLIN_ROUNDED_CLIP(effect);

    for (ClutterActor* child = clutter_actor_get_first_child(actor); child;
         child = clutter_actor_get_next_sibling(child)) {
        MetaGnoblinRoundedClip* clip = find_rounded_clip(child);
        if (clip)
            return clip;
    }
    return NULL;
}

void meta_gnoblin_window_effects_set_rounded_border(ClutterActor* actor, double width,
                                                    const double color[4]) {
    MetaGnoblinRoundedClip* clip;

    g_return_if_fail(CLUTTER_IS_ACTOR(actor));
    g_return_if_fail(color != NULL);
    if (!isfinite(width))
        return;
    for (guint i = 0; i < 4; i++) {
        if (!isfinite(color[i]))
            return;
    }

    clip = find_rounded_clip(actor);
    if (!clip)
        return;

    width = CLAMP(width, -40., 40.);
    double clamped_color[4];
    for (guint i = 0; i < G_N_ELEMENTS(clamped_color); i++)
        clamped_color[i] = CLAMP(color[i], 0., 1.);
    if (clip->border_width == width &&
        memcmp(clip->border_color, clamped_color, sizeof(clamped_color)) == 0)
        return;

    clip->border_width = width;
    memcpy(clip->border_color, clamped_color, sizeof(clamped_color));
    clutter_effect_queue_repaint(CLUTTER_EFFECT(clip));
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
