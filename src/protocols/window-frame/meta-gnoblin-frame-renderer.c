/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Native frame presentation/input and isolated, language-neutral renderers. */
#include "wayland/meta-gnoblin-frame-renderer.h"
#include "backends/meta-backend-private.h"
#include "clutter/clutter-pango.h"
#include "compositor/meta-surface-actor.h"
#include "config.h"
#include "core/window-private.h"
#include "gnoblin-window-frame-v1-server-protocol.h"
#include "meta/meta-wayland-client.h"
#include "meta/meta-window-actor.h"
#include "wayland/gnoblin-config.h"
#include "wayland/meta-wayland-actor-surface.h"
#include "wayland/meta-wayland-client-private.h"
#include "wayland/meta-wayland-filter-manager.h"
#include "wayland/meta-wayland-foreign-toplevel-common.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-window-wayland.h"
#include <math.h>
#include <string.h>

typedef struct _Frame Frame;
typedef struct _Renderer Renderer;
typedef struct {
    guint action;
    int x, y, width, height;
} Region;
typedef struct _FrameRole {
    MetaWaylandActorSurface parent;
    Frame* frame;
    guint32 ack, presented;
    GArray *pending, *committed;
} FrameRole;
typedef MetaWaylandActorSurfaceClass FrameRoleClass;
GType frame_role_get_type(void);
G_DEFINE_TYPE(FrameRole, frame_role, META_TYPE_WAYLAND_ACTOR_SURFACE)

struct _Renderer {
    char* name;
    char** argv;
    MetaWaylandClient* client;
    struct wl_resource* manager;
    guint retry, attempts;
    guint refs;
    gboolean retired;
};
struct _Frame {
    MetaWindow* window;
    ClutterActor *root, *fallback, *title, *buttons[3], *strips[4];
    MetaGnoblinFrameLayout layout;
    Renderer* renderer;
    struct wl_resource* resource;
    FrameRole* role; /* borrowed; role also has a weak frame pointer */
    guint32 serial, state;
    int width, height;
    guint pressed, hover;
    guint32 last_title_click_time;
    float last_title_click_x, last_title_click_y;
    guint actions[3], n_buttons;
    guint timeout;
    gboolean external;
    char* model_key;
    char* style;
};

static MetaWaylandCompositor* frame_compositor;
static GHashTable* renderers;
static GList* frames;
static gboolean stopping;
static void update_frame(Frame* frame);
static void offer_frame(Frame* frame);
static void start_renderer(Renderer* renderer);

/* Dependency-free vector icons: no font glyphs, icon theme, GTK or image
 * files. */
typedef struct {
    ClutterActor parent;
    Frame* frame;
    guint index;
    CoglPipeline* pipeline;
    CoglColor color;
} FrameButton;
typedef ClutterActorClass FrameButtonClass;
GType frame_button_get_type(void);
G_DEFINE_TYPE(FrameButton, frame_button, CLUTTER_TYPE_ACTOR)
static void button_paint(ClutterActor* actor, ClutterPaintNode* node,
                         ClutterPaintContext* context) {
    FrameButton* button = (FrameButton*)actor;
    Frame* frame = button->frame;
    if (!button->pipeline) {
        MetaBackend* backend =
            meta_context_get_backend(meta_wayland_compositor_get_context(frame_compositor));
        button->pipeline = cogl_pipeline_new(
            clutter_backend_get_cogl_context(meta_backend_get_clutter_backend(backend)));
        cogl_pipeline_set_layer_null_texture(button->pipeline, 0);
        /* Fragment output has transparency. Mark the pipeline non-opaque so
         * Cogl enables blending even though no image texture supplies alpha. */
        CoglColor translucent;
        cogl_color_init_from_4f(&translucent, 1, 1, 1, .5f);
        cogl_pipeline_set_color(button->pipeline, &translucent);
        CoglSnippet* snippet =
            cogl_snippet_new(COGL_SNIPPET_HOOK_FRAGMENT,
                             "uniform vec4 icon_color; uniform float button_size; uniform float "
                             "button_action; uniform float button_hover; uniform float "
                             "button_maximized;"
                             "float stroke(vec2 p,vec2 a,vec2 b){vec2 q=p-a;vec2 v=b-a;return "
                             "length(q-v*clamp(dot(q,v)/dot(v,v),0.,1.));}"
                             "float boxline(vec2 p,vec2 c,vec2 halfsize){vec2 "
                             "q=abs(p-c)-halfsize;return "
                             "abs(length(max(q,0.))+min(max(q.x,q.y),0.));}",
                             "vec2 p=(cogl_tex_coord_in[0].st-vec2(.5))*button_size;float d;"
                             "if(button_action<2.5)d=min(stroke(p,vec2(-4.,-4.),vec2(4.,4.)),"
                             "stroke(p,vec2(-4.,4.),vec2(4.,-4.)));"
                             "else "
                             "if(button_action<3.5){if(button_maximized>.5)d=min(boxline(p,vec2(-"
                             "1.,1.),vec2(3.5)),min(stroke(p,vec2(-1.,-5.),vec2(5.,-5.)),stroke("
                             "p,vec2(5.,-5.),vec2(5.,1.))));else d=boxline(p,vec2(0.),vec2(4.));}"
                             "else d=stroke(p,vec2(-4.,0.),vec2(4.,0.));"
                             "float ink=1.-smoothstep(.45,1.05,d);float "
                             "hover=(1.-smoothstep(11.5,12.5,length(p)))*button_hover;"
                             "float "
                             "alpha=(ink+hover*(1.-ink))*icon_color.a;cogl_color_out=vec4(icon_"
                             "color.rgb*alpha,alpha);");
        cogl_pipeline_add_snippet(button->pipeline, snippet);
        g_object_unref(snippet);
    }
    CoglPipeline* p = button->pipeline;
    guint action = frame->actions[button->index];
    gboolean enabled = action == 2   ? meta_window_can_close(frame->window)
                       : action == 3 ? meta_window_can_maximize(frame->window)
                                     : meta_window_can_minimize(frame->window);
    float color[] = {cogl_color_get_red(&button->color), cogl_color_get_green(&button->color),
                     cogl_color_get_blue(&button->color),
                     cogl_color_get_alpha(&button->color) * clutter_actor_get_paint_opacity(actor) /
                         255.f * (enabled ? 1.f : .4f)};
    cogl_pipeline_set_uniform_float(p, cogl_pipeline_get_uniform_location(p, "icon_color"), 4, 1,
                                    color);
    cogl_pipeline_set_uniform_1f(p, cogl_pipeline_get_uniform_location(p, "button_size"),
                                 clutter_actor_get_width(actor));
    cogl_pipeline_set_uniform_1f(p, cogl_pipeline_get_uniform_location(p, "button_action"), action);
    cogl_pipeline_set_uniform_1f(p, cogl_pipeline_get_uniform_location(p, "button_maximized"),
                                 meta_window_is_maximized(frame->window));
    cogl_pipeline_set_uniform_1f(
        p, cogl_pipeline_get_uniform_location(p, "button_hover"),
        enabled && frame->hover == action ? (frame->pressed == action ? .22f : .12f) : 0.f);
    ClutterPaintNode* paint = clutter_pipeline_node_new(p);
    clutter_paint_node_add_rectangle(paint, &(ClutterActorBox){0, 0, clutter_actor_get_width(actor),
                                                               clutter_actor_get_height(actor)});
    clutter_paint_node_add_child(node, paint);
    clutter_paint_node_unref(paint);
}
static void button_dispose(GObject* object) {
    g_clear_object(&((FrameButton*)object)->pipeline);
    G_OBJECT_CLASS(frame_button_parent_class)->dispose(object);
}
static void frame_button_init(FrameButton* button) {}
static void frame_button_class_init(FrameButtonClass* klass) {
    klass->paint_node = button_paint;
    G_OBJECT_CLASS(klass)->dispose = button_dispose;
}
static void redraw_buttons(Frame* frame) {
    for (int i = 0; i < 3; i++)
        clutter_actor_queue_redraw(frame->buttons[i]);
}

/* Masks untrusted frame pixels out of the application's content. Both native
 * and external rendering get the same outer rounded rectangle. */
typedef struct {
    ClutterOffscreenEffect parent;
    Frame* frame;
} FrameMask;
typedef ClutterOffscreenEffectClass FrameMaskClass;
GType frame_mask_get_type(void);
G_DEFINE_TYPE(FrameMask, frame_mask, CLUTTER_TYPE_OFFSCREEN_EFFECT)

static CoglPipeline* mask_pipeline(ClutterOffscreenEffect* effect, CoglTexture* texture) {
    CoglPipeline* pipeline =
        CLUTTER_OFFSCREEN_EFFECT_CLASS(frame_mask_parent_class)->create_pipeline(effect, texture);
    CoglSnippet* snippet =
        cogl_snippet_new(COGL_SNIPPET_HOOK_FRAGMENT,
                         "uniform vec2 frame_size; uniform vec4 frame_content; uniform float "
                         "frame_radius; uniform float frame_exponent; uniform vec4 "
                         "frame_texture;",
                         "vec2 p = frame_texture.xy + cogl_tex_coord_in[0].st * frame_texture.zw;"
                         "float inside = step(frame_content.x,p.x)*step(frame_content.y,p.y)*"
                         "step(p.x,frame_content.z)*step(p.y,frame_content.w);"
                         /* The content hole has concentric bottom corners, not a rectangular
                          * cutout. Keep the header/content join square. Use separate inset radii
                          * so unequal side and bottom extents do not change the outer corner
                          * centre. */
                         "float side = p.x < frame_size.x*0.5 ? frame_content.x : "
                         "frame_size.x-frame_content.z;"
                         "vec2 ir = max(vec2(frame_radius)-vec2(side,"
                         "frame_size.y-frame_content.w),vec2(0.0));"
                         "vec2 centre = vec2(p.x < frame_size.x*0.5 ? "
                         "frame_content.x+ir.x : frame_content.z-ir.x,frame_content.w-ir.y);"
                         "vec2 iq = vec2(abs(p.x-centre.x),p.y-centre.y);"
                         "bool corner = p.y > centre.y && (p.x < frame_content.x+ir.x || "
                         "p.x > frame_content.z-ir.x);"
                         "if (corner && min(ir.x,ir.y)>0.0) {"
                         "vec2 n=iq/max(ir,vec2(0.001));"
                         "float id=pow(pow(n.x,frame_exponent)+pow(n.y,frame_exponent),"
                         "1.0/frame_exponent);"
                         "float aa=0.5/min(ir.x,ir.y);"
                         "inside *= 1.0-smoothstep(1.0-aa,1.0+aa,id); }"
                         "vec2 q = "
                         "max(abs(p-frame_size*0.5)-(frame_size*0.5-vec2(frame_radius)),vec2(0.0)"
                         ");"
                         "float r = max(frame_radius,0.001);"
                         "float d = "
                         "pow(pow(q.x/r,frame_exponent)+pow(q.y/r,frame_exponent),1.0/"
                         "frame_exponent)*r;"
                         "float mask = frame_radius < 0.5 ? 1.0 : 1.0-smoothstep(r-0.5,r+0.5,d);"
                         "cogl_color_out *= (1.0-inside)*mask;");
    cogl_pipeline_add_snippet(pipeline, snippet);
    g_object_unref(snippet);
    return pipeline;
}

static void mask_paint(ClutterOffscreenEffect* effect, ClutterPaintNode* node,
                       ClutterPaintContext* context) {
    Frame* frame = ((FrameMask*)effect)->frame;
    CoglPipeline* pipeline = clutter_offscreen_effect_get_pipeline(effect);
    float size[] = {frame->width, frame->height};
    graphene_rect_t target;
    clutter_offscreen_effect_get_target_rect(effect, &target);
    float texture[] = {target.origin.x, target.origin.y, target.size.width, target.size.height};
    cogl_pipeline_set_uniform_float(
        pipeline, cogl_pipeline_get_uniform_location(pipeline, "frame_texture"), 4, 1, texture);
    float content[] = {frame->layout.border[3], frame->layout.border[0],
                       frame->width - frame->layout.border[1],
                       frame->height - frame->layout.border[2]};
    GVariant* options = g_object_get_data(G_OBJECT(frame->window), "gnoblin-frame-style");
    double radius = 0, exponent = 2;
    if (options) {
        g_variant_lookup(options, "radius", "d", &radius);
        g_variant_lookup(options, "exponent", "d", &exponent);
    }
    cogl_pipeline_set_uniform_float(
        pipeline, cogl_pipeline_get_uniform_location(pipeline, "frame_size"), 2, 1, size);
    cogl_pipeline_set_uniform_float(
        pipeline, cogl_pipeline_get_uniform_location(pipeline, "frame_content"), 4, 1, content);
    cogl_pipeline_set_uniform_1f(pipeline,
                                 cogl_pipeline_get_uniform_location(pipeline, "frame_radius"),
                                 (float)MIN(radius, MIN(frame->width, frame->height) / 2.0));
    cogl_pipeline_set_uniform_1f(pipeline,
                                 cogl_pipeline_get_uniform_location(pipeline, "frame_exponent"),
                                 (float)CLAMP(exponent, 2, 6));
    CLUTTER_OFFSCREEN_EFFECT_CLASS(frame_mask_parent_class)->paint_target(effect, node, context);
}
static void frame_mask_init(FrameMask* mask) {}
static void frame_mask_class_init(FrameMaskClass* klass) {
    klass->create_pipeline = mask_pipeline;
    klass->paint_target = mask_paint;
}

static void fallback(Frame* frame) {
    frame->external = FALSE;
    clutter_actor_show(frame->fallback);
    if (frame->role) {
        MetaSurfaceActor* actor =
            meta_wayland_actor_surface_get_actor(META_WAYLAND_ACTOR_SURFACE(frame->role));
        if (actor)
            clutter_actor_hide(CLUTTER_ACTOR(actor));
    }
}

static gboolean renderer_timeout(gpointer data) {
    Frame* frame = data;
    frame->timeout = 0;
    fallback(frame);
    return G_SOURCE_REMOVE;
}

static guint hit_action(Frame* frame, float x, float y) {
    int* b = frame->layout.border;
    if (x < 0 || y < 0 || x >= frame->width || y >= frame->height ||
        (x >= b[3] && y >= b[0] && x < frame->width - b[1] && y < frame->height - b[2]))
        return 0;
    GVariant* options = g_object_get_data(G_OBJECT(frame->window), "gnoblin-frame-style");
    double radius = 0, exponent = 2;
    if (options) {
        g_variant_lookup(options, "radius", "d", &radius);
        g_variant_lookup(options, "exponent", "d", &exponent);
    }
    radius = CLAMP(radius, 0, MIN(frame->width, frame->height) / 2.0);
    exponent = CLAMP(exponent, 2, 6);
    if (radius > 0) {
        double qx = MAX(fabs(x - frame->width / 2.0) - (frame->width / 2.0 - radius), 0) / radius;
        double qy = MAX(fabs(y - frame->height / 2.0) - (frame->height / 2.0 - radius), 0) / radius;
        if (pow(qx, exponent) + pow(qy, exponent) > 1)
            return 0;
    }
    if (frame->external && frame->role) {
        GArray* regions = frame->role->committed;
        for (int i = regions->len - 1; i >= 0; i--) {
            Region* r = &g_array_index(regions, Region, i);
            if (x >= r->x && y >= r->y && x < r->x + r->width && y < r->y + r->height)
                return r->action;
        }
        return 0;
    }
    if (y >= frame->height - b[2])
        return 9;
    if (x < b[3])
        return 11;
    if (x >= frame->width - b[1])
        return 7;
    if (y < MIN(4, b[0]))
        return 5;
    if (y < b[0]) {
        int button = (int)((frame->width - b[1] - x) / MAX(1, b[0]));
        if (button < frame->n_buttons)
            return frame->actions[button];
        return 1;
    }
    return 0;
}

static gboolean frame_event(ClutterActor* actor, ClutterEvent* event, gpointer data) {
    Frame* frame = data;
    float sx, sy, x, y;
    guint action;
    ClutterEventType type = clutter_event_type(event);
    if (type == CLUTTER_LEAVE) {
        frame->hover = 0;
        redraw_buttons(frame);
        if (frame->external && frame->resource)
            gnoblin_window_frame_v1_send_interaction(frame->resource, 0, !!frame->pressed);
        return CLUTTER_EVENT_PROPAGATE;
    }
    if (type != CLUTTER_BUTTON_PRESS && type != CLUTTER_BUTTON_RELEASE && type != CLUTTER_MOTION)
        return CLUTTER_EVENT_PROPAGATE;
    clutter_event_get_coords(event, &sx, &sy);
    clutter_actor_transform_stage_point(frame->root, sx, sy, &x, &y);
    action = hit_action(frame, x, y);
    if (!action && type != CLUTTER_BUTTON_RELEASE)
        return CLUTTER_EVENT_PROPAGATE;
    if (type == CLUTTER_MOTION) {
        if (frame->hover != action && frame->external && frame->resource)
            gnoblin_window_frame_v1_send_interaction(frame->resource, action, !!frame->pressed);
        frame->hover = action;
        redraw_buttons(frame);
        return CLUTTER_EVENT_STOP;
    }
    if (type == CLUTTER_BUTTON_PRESS && action == 1 &&
        clutter_event_get_button(event) == CLUTTER_BUTTON_SECONDARY && !frame->window->unmanaging) {
        meta_window_show_menu(frame->window, META_WINDOW_MENU_WM, (int)sx, (int)sy);
        return CLUTTER_EVENT_STOP;
    }
    if (clutter_event_get_button(event) != CLUTTER_BUTTON_PRIMARY)
        return CLUTTER_EVENT_PROPAGATE;
    if (frame->window->unmanaging)
        return CLUTTER_EVENT_STOP;
    if (type == CLUTTER_BUTTON_PRESS) {
        if (action == 1) {
            guint32 now = clutter_event_get_time(event);
            gboolean double_click = frame->last_title_click_time &&
                                    now - frame->last_title_click_time <= 400 &&
                                    fabs(x - frame->last_title_click_x) <= 8 &&
                                    fabs(y - frame->last_title_click_y) <= 8;
            frame->last_title_click_time = double_click ? 0 : now;
            frame->last_title_click_x = x;
            frame->last_title_click_y = y;
            if (double_click) {
                if (meta_window_can_maximize(frame->window)) {
                    if (meta_window_is_maximized(frame->window))
                        meta_window_unmaximize(frame->window);
                    else
                        meta_window_maximize(frame->window);
                }
                return CLUTTER_EVENT_STOP;
            }
        } else {
            frame->last_title_click_time = 0;
        }
        frame->pressed = action;
        meta_window_activate(frame->window, clutter_event_get_time(event));
        if (action == 1 || action >= 5) {
            static const MetaGrabOp resize[] = {META_GRAB_OP_RESIZING_N, META_GRAB_OP_RESIZING_NE,
                                                META_GRAB_OP_RESIZING_E, META_GRAB_OP_RESIZING_SE,
                                                META_GRAB_OP_RESIZING_S, META_GRAB_OP_RESIZING_SW,
                                                META_GRAB_OP_RESIZING_W, META_GRAB_OP_RESIZING_NW};
            MetaContext* context = meta_wayland_compositor_get_context(frame_compositor);
            MetaBackend* backend = meta_context_get_backend(context);
            ClutterBackend* cb = meta_backend_get_clutter_backend(backend);
            ClutterStage* stage = CLUTTER_STAGE(clutter_actor_get_stage(actor));
            ClutterSprite* sprite = clutter_backend_get_sprite(cb, stage, event);
            graphene_point_t point = GRAPHENE_POINT_INIT(sx, sy);
            if ((action == 1 && meta_window_allows_move(frame->window)) ||
                (action >= 5 && action <= 12 && meta_window_allows_resize(frame->window)))
                meta_window_begin_grab_op(frame->window,
                                          action == 1 ? META_GRAB_OP_MOVING : resize[action - 5],
                                          sprite, clutter_event_get_time(event), &point);
        }
    } else {
        if (action == frame->pressed) {
            if (action == 2 && meta_window_can_close(frame->window))
                meta_window_delete(frame->window, clutter_event_get_time(event));
            if (action == 3 && meta_window_can_maximize(frame->window)) {
                if (meta_window_is_maximized(frame->window))
                    meta_window_unmaximize(frame->window);
                else
                    meta_window_maximize(frame->window);
            }
            if (action == 4 && meta_window_can_minimize(frame->window))
                meta_window_minimize(frame->window);
        }
        frame->pressed = 0;
    }
    if (frame->external && frame->resource)
        gnoblin_window_frame_v1_send_interaction(frame->resource, action, !!frame->pressed);
    redraw_buttons(frame);
    return CLUTTER_EVENT_STOP;
}

static void detach_frame(Frame* frame) {
    if (frame->timeout) {
        g_source_remove(frame->timeout);
        frame->timeout = 0;
    }
    if (frame->resource) {
        gnoblin_window_frame_v1_send_closed(frame->resource);
        wl_resource_set_user_data(frame->resource, NULL);
        frame->resource = NULL;
    }
    if (frame->role) {
        ClutterActor* actor = CLUTTER_ACTOR(
            meta_wayland_actor_surface_get_actor(META_WAYLAND_ACTOR_SURFACE(frame->role)));
        frame->role->frame = NULL;
        frame->role = NULL;
        if (actor && clutter_actor_get_parent(actor))
            clutter_actor_remove_child(clutter_actor_get_parent(actor), actor);
    }
    fallback(frame);
    g_clear_pointer(&frame->model_key, g_free);
}

static void frame_destroyed(ClutterActor* actor, gpointer data) {
    Frame* frame = data;
    frames = g_list_remove(frames, frame);
    g_signal_handlers_disconnect_by_data(frame->window, frame);
    g_object_set_data(G_OBJECT(frame->window), "gnoblin-native-frame", NULL);
    detach_frame(frame);
    g_object_unref(frame->window);
    g_free(frame->model_key);
    g_free(frame->style);
    g_free(frame);
}

static void window_changed(MetaWindow* window, GParamSpec* pspec, gpointer data) {
    update_frame(data);
}

GVariant* meta_gnoblin_frame_renderer_status(MetaWindow* window) {
    Frame* frame = g_object_get_data(G_OBJECT(window), "gnoblin-native-frame");
    GVariantBuilder b;
    g_variant_builder_init(&b, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&b, "{sv}", "external", g_variant_new_boolean(frame && frame->external));
    g_variant_builder_add(&b, "{sv}", "visible",
                          g_variant_new_boolean(frame && clutter_actor_is_visible(frame->root)));
    g_variant_builder_add(
        &b, "{sv}", "pid",
        g_variant_new_int32(frame && frame->renderer && frame->renderer->client
                                ? meta_wayland_client_get_pid(frame->renderer->client)
                                : 0));
    g_variant_builder_add(&b, "{sv}", "serial", g_variant_new_uint32(frame ? frame->serial : 0));
    g_variant_builder_add(&b, "{sv}", "hover",
                          g_variant_new_uint32(frame ? frame->hover : 0));
    g_variant_builder_add(&b, "{sv}", "pressed",
                          g_variant_new_uint32(frame ? frame->pressed : 0));
    GVariantBuilder regions;
    g_variant_builder_init(&regions, G_VARIANT_TYPE("a(uiiii)"));
    if (frame && frame->external && frame->role) {
        for (guint i = 0; i < frame->role->committed->len; i++) {
            Region* r = &g_array_index(frame->role->committed, Region, i);
            g_variant_builder_add(&regions, "(uiiii)", r->action, r->x, r->y, r->width, r->height);
        }
    } else if (frame)
        for (guint i = 0; i < frame->n_buttons; i++) {
            int top = frame->layout.border[0];
            g_variant_builder_add(&regions, "(uiiii)", frame->actions[i],
                                  frame->width - frame->layout.border[1] - (i + 1) * top, 0, top,
                                  top);
        }
    g_variant_builder_add(&b, "{sv}", "regions", g_variant_builder_end(&regions));
    return g_variant_builder_end(&b);
}

static Frame* new_frame(MetaWindow* window) {
    ClutterActor* parent = CLUTTER_ACTOR(meta_window_get_compositor_private(window));
    Frame* frame = g_new0(Frame, 1);
    frame->window = g_object_ref(window);
    frame->root = clutter_actor_new();
    clutter_actor_set_name(frame->root, "gnoblin-native-frame");
    frame->fallback = clutter_actor_new();
    clutter_actor_add_child(frame->root, frame->fallback);
    frame->title = clutter_text_new_with_text("Sans 10", "");
    clutter_actor_add_child(frame->fallback, frame->title);
    for (int i = 0; i < 3; i++) {
        frame->buttons[i] = g_object_new(frame_button_get_type(), NULL);
        ((FrameButton*)frame->buttons[i])->frame = frame;
        ((FrameButton*)frame->buttons[i])->index = i;
        clutter_actor_add_child(frame->fallback, frame->buttons[i]);
    }
    for (int i = 0; i < 4; i++) {
        frame->strips[i] = clutter_actor_new();
        clutter_actor_set_reactive(frame->strips[i], TRUE);
        g_signal_connect(frame->strips[i], "event", G_CALLBACK(frame_event), frame);
        clutter_actor_add_child(frame->root, frame->strips[i]);
    }
    FrameMask* mask = g_object_new(frame_mask_get_type(), NULL);
    mask->frame = frame;
    clutter_actor_add_effect(frame->root, CLUTTER_EFFECT(mask));
    clutter_actor_add_child(parent, frame->root);
    g_signal_connect(frame->root, "destroy", G_CALLBACK(frame_destroyed), frame);
    g_signal_connect(window, "notify::title", G_CALLBACK(window_changed), frame);
    g_signal_connect(window, "notify::appears-focused", G_CALLBACK(window_changed), frame);
    g_object_set_data(G_OBJECT(window), "gnoblin-native-frame", frame);
    frames = g_list_prepend(frames, frame);
    return frame;
}

static void update_frame(Frame* frame) {
    int previous_width = frame->width, previous_height = frame->height;
    MtkRectangle rect, buffer;
    const char *name = "native", *style = "default", *bg = "#242424", *fg = "#eeeeee";
    GVariant* options = g_object_get_data(G_OBJECT(frame->window), "gnoblin-frame-style");
    Renderer* renderer;
    int scale = meta_window_wayland_get_geometry_scale(frame->window);
    int* b = frame->layout.border;
    if (options) {
        g_variant_lookup(options, "renderer", "&s", &name);
        g_variant_lookup(options, "style", "&s", &style);
        g_variant_lookup(
            options, meta_window_has_focus(frame->window) ? "background" : "inactive-background",
            "&s", &bg);
        g_variant_lookup(options, "foreground", "&s", &fg);
    }
    renderer = renderers ? g_hash_table_lookup(renderers, name) : NULL;
    if (renderer != frame->renderer) {
        detach_frame(frame);
        frame->renderer = renderer;
    }
    meta_window_get_frame_rect(frame->window, &rect);
    meta_window_get_buffer_rect(frame->window, &buffer);
    frame->width = rect.width / scale;
    frame->height = rect.height / scale;
    if (frame->width <= 0 || frame->height <= 0)
        return;
    clutter_actor_set_position(frame->root, (rect.x - buffer.x) / (float)scale,
                               (rect.y - buffer.y) / (float)scale);
    clutter_actor_set_size(frame->root, frame->width, frame->height);
    clutter_actor_set_clip(frame->root, 0, 0, frame->width, frame->height);
    gboolean visible = b[0] || b[1] || b[2] || b[3];
    if (!visible) {
        clutter_actor_hide(frame->root);
        return;
    }
    clutter_actor_show(frame->root);
    CoglColor background, foreground;
    if (!cogl_color_from_string(&background, bg))
        cogl_color_from_string(&background, "#242424");
    if (!cogl_color_from_string(&foreground, fg))
        cogl_color_from_string(&foreground, "#eeeeee");
    g_autoptr(GVariant) buttons =
        options ? g_variant_lookup_value(options, "button-layout", G_VARIANT_TYPE_STRING_ARRAY)
                : NULL;
    frame->n_buttons = buttons ? MIN(3, g_variant_n_children(buttons)) : 3;
    for (guint i = 0; i < frame->n_buttons; i++) {
        const char* button = NULL;
        if (buttons)
            g_variant_get_child(buttons, frame->n_buttons - 1 - i, "&s", &button);
        frame->actions[i] = button ? (!strcmp(button, "close")      ? 2
                                      : !strcmp(button, "maximize") ? 3
                                                                    : 4)
                                   : 2 + i;
    }
    clutter_actor_set_background_color(frame->fallback, &background);
    clutter_actor_set_size(frame->fallback, frame->width, frame->height);
    clutter_text_set_color(CLUTTER_TEXT(frame->title), &foreground);
    clutter_text_set_text(CLUTTER_TEXT(frame->title), meta_window_get_title(frame->window) ?: "");
    clutter_text_set_ellipsize(CLUTTER_TEXT(frame->title), PANGO_ELLIPSIZE_END);
    clutter_actor_set_position(frame->title, b[3] + 10, MAX(0, (b[0] - 18) / 2));
    clutter_actor_set_size(
        frame->title, MAX(0, frame->width - b[3] - b[1] - (int)frame->n_buttons * b[0] - 20), 18);
    for (int i = 0; i < 3; i++) {
        if (i >= frame->n_buttons) {
            clutter_actor_hide(frame->buttons[i]);
            continue;
        }
        clutter_actor_show(frame->buttons[i]);
        ((FrameButton*)frame->buttons[i])->color = foreground;
        clutter_actor_set_size(frame->buttons[i], b[0], b[0]);
        clutter_actor_set_position(frame->buttons[i], frame->width - b[1] - (i + 1) * b[0], 0);
        clutter_actor_queue_redraw(frame->buttons[i]);
    }
    int boxes[4][4] = {{0, 0, frame->width, b[0]},
                       {frame->width - b[1], b[0], b[1], frame->height - b[0]},
                       {0, frame->height - b[2], frame->width, b[2]},
                       {0, b[0], b[3], frame->height - b[0]}};
    for (int i = 0; i < 4; i++) {
        clutter_actor_set_position(frame->strips[i], boxes[i][0], boxes[i][1]);
        clutter_actor_set_size(frame->strips[i], MAX(0, boxes[i][2]), MAX(0, boxes[i][3]));
    }
    frame->state = (meta_window_has_focus(frame->window) ? 1 : 0) |
                   (meta_window_is_maximized(frame->window) ? 2 : 0) |
                   (meta_window_can_close(frame->window) ? 4 : 0) |
                   (meta_window_can_maximize(frame->window) ? 8 : 0) |
                   (meta_window_can_minimize(frame->window) ? 16 : 0) |
                   (meta_window_allows_resize(frame->window) ? 32 : 0);
    g_free(frame->style);
    frame->style = g_strdup(style);
    if (renderer && !renderer->client && !renderer->retry && renderer->attempts < 4)
        start_renderer(renderer);
    if (renderer && renderer->manager && !frame->resource)
        offer_frame(frame);
    if (frame->resource) {
        const char* title = meta_window_get_title(frame->window) ?: "";
        const char* app = meta_gnoblin_foreign_toplevel_window_app_id(frame->window) ?: "";
        g_autofree char* key =
            g_strdup_printf("%d,%d,%d,%d,%d,%d,%u:%s:%s:%s", frame->width, frame->height, b[0],
                            b[1], b[2], b[3], frame->state, title, app, style);
        if (g_strcmp0(key, frame->model_key)) {
            g_free(frame->model_key);
            frame->model_key = g_steal_pointer(&key);
            frame->serial = wl_display_next_serial(frame_compositor->wayland_display);
            /* A matching old-size buffer must not be stretched over a new
             * layout. */
            if (frame->width != previous_width || frame->height != previous_height)
                fallback(frame);
            gnoblin_window_frame_v1_send_configure(frame->resource, frame->serial, frame->width,
                                                   frame->height, b[0], b[1], b[2], b[3],
                                                   frame->state, title, app, style);
            if (frame->timeout)
                g_source_remove(frame->timeout);
            frame->timeout = g_timeout_add(1000, renderer_timeout, frame);
        }
    }
    clutter_actor_queue_redraw(frame->root);
}

void meta_gnoblin_frame_renderer_sync(MetaWindow* window, const MetaGnoblinFrameLayout* layout) {
    Frame* frame = g_object_get_data(G_OBJECT(window), "gnoblin-native-frame");
    if (!frame &&
        !(layout->border[0] || layout->border[1] || layout->border[2] || layout->border[3]))
        return;
    if (!frame)
        frame = new_frame(window);
    frame->layout = *layout;
    update_frame(frame);
}
void meta_gnoblin_frame_renderer_style_changed(MetaWindow* window) {
    Frame* frame = g_object_get_data(G_OBJECT(window), "gnoblin-native-frame");
    if (frame)
        update_frame(frame);
}

static int role_scale(MetaWaylandActorSurface* role) {
    return 1;
}
static void role_commit(MetaWaylandSurfaceRole* role, MetaWaylandTransaction* transaction,
                        MetaWaylandSurfaceState* pending) {
    FrameRole* self = (FrameRole*)role;
    GVariantBuilder regions;
    g_variant_builder_init(&regions, G_VARIANT_TYPE("a(uiiii)"));
    for (guint i = 0; i < self->pending->len; i++) {
        Region* r = &g_array_index(self->pending, Region, i);
        g_variant_builder_add(&regions, "(uiiii)", r->action, r->x, r->y, r->width, r->height);
    }
    GVariant* snapshot = g_variant_ref_sink(
        g_variant_new("(u@a(uiiii))", self->ack, g_variant_builder_end(&regions)));
    g_object_set_data_full(G_OBJECT(pending), "gnoblin-frame-commit", snapshot,
                           (GDestroyNotify)g_variant_unref);
}
void meta_gnoblin_frame_renderer_merge_state(MetaWaylandSurfaceState* from,
                                             MetaWaylandSurfaceState* to) {
    GVariant* snapshot = g_object_get_data(G_OBJECT(from), "gnoblin-frame-commit");
    if (snapshot)
        g_object_set_data_full(G_OBJECT(to), "gnoblin-frame-commit", g_variant_ref(snapshot),
                               (GDestroyNotify)g_variant_unref);
}
static void role_sync(MetaWaylandActorSurface* role) {
    META_WAYLAND_ACTOR_SURFACE_CLASS(frame_role_parent_class)->sync_actor_state(role);
    MetaSurfaceActor* actor = meta_wayland_actor_surface_get_actor(role);
    g_autoptr(MtkRegion) empty = mtk_region_create();
    meta_surface_actor_set_opaque_region(actor, NULL);
    meta_surface_actor_set_input_region(actor, empty);
    clutter_actor_set_reactive(CLUTTER_ACTOR(actor), FALSE);
}
static void role_post_apply(MetaWaylandSurfaceRole* role, MetaWaylandSurfaceState* pending) {
    FrameRole* self = (FrameRole*)role;
    Frame* frame = self->frame;
    MetaWaylandSurface* surface = meta_wayland_surface_role_get_surface(role);
    if (!frame)
        return;
    GVariant* snapshot = g_object_get_data(G_OBJECT(pending), "gnoblin-frame-commit");
    guint32 ack = 0;
    g_autoptr(GVariant) regions = NULL;
    if (snapshot)
        g_variant_get(snapshot, "(u@a(uiiii))", &ack, &regions);
    if (!meta_wayland_surface_get_buffer(surface)) {
        fallback(frame);
        return;
    }
    /* An older focus/title repaint may arrive after another configure. Keep
     * the presented frame until the latest repaint or the renderer timeout. */
    if (ack != frame->serial)
        return;
    if (meta_wayland_surface_get_width(surface) != frame->width ||
        meta_wayland_surface_get_height(surface) != frame->height) {
        fallback(frame);
        wl_resource_post_error(frame->resource, 1,
                               "Frame buffer dimensions do not match configure");
        return;
    }
    g_array_set_size(self->committed, 0);
    GVariantIter iter;
    Region r;
    g_variant_iter_init(&iter, regions);
    while (g_variant_iter_next(&iter, "(uiiii)", &r.action, &r.x, &r.y, &r.width, &r.height))
        g_array_append_val(self->committed, r);
    self->presented = ack;
    frame->external = TRUE;
    if (frame->timeout) {
        g_source_remove(frame->timeout);
        frame->timeout = 0;
    }
    clutter_actor_hide(frame->fallback);
    clutter_actor_show(
        CLUTTER_ACTOR(meta_wayland_actor_surface_get_actor(META_WAYLAND_ACTOR_SURFACE(role))));
    clutter_actor_queue_redraw(frame->root);
}
static void role_dispose(GObject* object) {
    FrameRole* self = (FrameRole*)object;
    if (self->frame) {
        Frame* frame = self->frame;
        ClutterActor* actor =
            CLUTTER_ACTOR(meta_wayland_actor_surface_get_actor(META_WAYLAND_ACTOR_SURFACE(self)));
        fallback(frame);
        if (actor && clutter_actor_get_parent(actor))
            clutter_actor_remove_child(clutter_actor_get_parent(actor), actor);
        self->frame = NULL;
        frame->role = NULL;
        clutter_actor_queue_redraw(frame->root);
    }
    G_OBJECT_CLASS(frame_role_parent_class)->dispose(object);
}
static void role_finalize(GObject* object) {
    FrameRole* self = (FrameRole*)object;
    g_array_unref(self->pending);
    g_array_unref(self->committed);
    G_OBJECT_CLASS(frame_role_parent_class)->finalize(object);
}
static void frame_role_init(FrameRole* self) {
    self->pending = g_array_new(FALSE, FALSE, sizeof(Region));
    self->committed = g_array_new(FALSE, FALSE, sizeof(Region));
}
static void frame_role_class_init(FrameRoleClass* klass) {
    klass->get_geometry_scale = role_scale;
    klass->sync_actor_state = role_sync;
    META_WAYLAND_SURFACE_ROLE_CLASS(klass)->post_apply_state = role_post_apply;
    META_WAYLAND_SURFACE_ROLE_CLASS(klass)->commit_state = role_commit;
    G_OBJECT_CLASS(klass)->dispose = role_dispose;
    G_OBJECT_CLASS(klass)->finalize = role_finalize;
}

static void destroy_resource(struct wl_client* client, struct wl_resource* resource) {
    wl_resource_destroy(resource);
}
static void attach_surface(struct wl_client* client, struct wl_resource* resource,
                           struct wl_resource* surface_resource) {
    Frame* frame = wl_resource_get_user_data(resource);
    MetaWaylandSurface* surface = wl_resource_get_user_data(surface_resource);
    if (!frame)
        return;
    if (frame->role || surface->role ||
        !meta_wayland_surface_assign_role(surface, frame_role_get_type(), NULL)) {
        wl_resource_post_error(resource, 0, "Frame requires an unroled surface");
        return;
    }
    frame->role = (FrameRole*)surface->role;
    frame->role->frame = frame;
    ClutterActor* actor = CLUTTER_ACTOR(
        meta_wayland_actor_surface_get_actor(META_WAYLAND_ACTOR_SURFACE(frame->role)));
    clutter_actor_insert_child_at_index(frame->root, actor, 1);
    clutter_actor_hide(actor);
}
static void ack_configure(struct wl_client* client, struct wl_resource* resource, uint32_t serial) {
    Frame* frame = wl_resource_get_user_data(resource);
    if (!frame)
        return;
    if (!frame->role || serial != frame->serial) {
        /* Old events may already be queued when a new configure is emitted. */
        if (frame->role && (int32_t)(serial - frame->serial) < 0) {
            frame->role->ack = serial;
            return;
        }
        wl_resource_post_error(resource, 1, "Unknown frame configure serial");
        return;
    }
    frame->role->ack = serial;
}
static void clear_regions(struct wl_client* client, struct wl_resource* resource) {
    Frame* f = wl_resource_get_user_data(resource);
    if (f && f->role)
        g_array_set_size(f->role->pending, 0);
}
static void region(struct wl_client* client, struct wl_resource* resource, uint32_t action,
                   int32_t x, int32_t y, int32_t w, int32_t h) {
    Frame* frame = wl_resource_get_user_data(resource);
    if (!frame)
        return;
    if (!frame->role || action < 1 || action > 12 || x < 0 || y < 0 || w < 0 || h < 0 ||
        x > 32768 || y > 32768 || w > 32768 || h > 32768 || frame->role->pending->len >= 64) {
        wl_resource_post_error(resource, 1, "Invalid frame region");
        return;
    }
    Region r = {action, x, y, w, h};
    g_array_append_val(frame->role->pending, r);
}
static const struct gnoblin_window_frame_v1_interface frame_impl = {
    destroy_resource, attach_surface, ack_configure, clear_regions, region};
static void frame_resource_destroyed(struct wl_resource* resource) {
    Frame* frame = wl_resource_get_user_data(resource);
    if (frame) {
        frame->resource = NULL;
        detach_frame(frame);
    }
}
static void offer_frame(Frame* frame) {
    frame->resource = wl_resource_create(wl_resource_get_client(frame->renderer->manager),
                                         &gnoblin_window_frame_v1_interface, 1, 0);
    if (!frame->resource) {
        wl_client_post_no_memory(wl_resource_get_client(frame->renderer->manager));
        return;
    }
    wl_resource_set_implementation(frame->resource, &frame_impl, frame, frame_resource_destroyed);
    gnoblin_window_frame_manager_v1_send_frame(frame->renderer->manager, frame->resource);
}
static void manager_destroyed(struct wl_resource* resource) {
    Renderer* renderer = wl_resource_get_user_data(resource);
    renderer->manager = NULL;
    for (GList* l = frames; l; l = l->next) {
        Frame* frame = l->data;
        if (frame->renderer == renderer)
            detach_frame(frame);
    }
}
static const struct gnoblin_window_frame_manager_v1_interface manager_impl = {destroy_resource};
static Renderer* renderer_for_client(const struct wl_client* client) {
    GHashTableIter iter;
    gpointer value;
    if (!renderers || stopping)
        return NULL;
    g_hash_table_iter_init(&iter, renderers);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        Renderer* r = value;
        if (r->client && meta_get_wayland_client(client) == r->client)
            return r;
    }
    return NULL;
}
static MetaWaylandAccess filter(const struct wl_client* client, const struct wl_global* global,
                                gpointer data) {
    return renderer_for_client(client) ? META_WAYLAND_ACCESS_ALLOWED : META_WAYLAND_ACCESS_DENIED;
}
static void bind_manager(struct wl_client* client, void* data, uint32_t version, uint32_t id) {
    Renderer* renderer = renderer_for_client(client);
    if (!renderer) {
        wl_client_post_implementation_error(client, "Not a frame renderer");
        return;
    }
    if (renderer->manager) {
        wl_client_post_implementation_error(client, "Duplicate frame manager");
        return;
    }
    renderer->manager =
        wl_resource_create(client, &gnoblin_window_frame_manager_v1_interface, 1, id);
    if (!renderer->manager) {
        wl_client_post_no_memory(client);
        return;
    }
    wl_resource_set_implementation(renderer->manager, &manager_impl, renderer, manager_destroyed);
    for (GList* l = frames; l; l = l->next) {
        Frame* frame = l->data;
        if (frame->renderer == renderer)
            update_frame(frame);
    }
}
static gboolean retry_renderer(gpointer data) {
    Renderer* r = data;
    r->retry = 0;
    start_renderer(r);
    return G_SOURCE_REMOVE;
}
static void renderer_unref(gpointer data) {
    Renderer* renderer = data;
    if (--renderer->refs)
        return;
    g_strfreev(renderer->argv);
    g_free(renderer->name);
    g_free(renderer);
}
static void renderer_exited(GObject* process, GAsyncResult* result, gpointer data) {
    Renderer* renderer = data;
    g_subprocess_wait_finish(G_SUBPROCESS(process), result, NULL);
    if (renderer->client) {
        meta_wayland_client_destroy(renderer->client);
        g_clear_object(&renderer->client);
    }
    if (!stopping && !renderer->retired && renderer->attempts < 4)
        renderer->retry = g_timeout_add(250u << renderer->attempts, retry_renderer, renderer);
    renderer_unref(renderer);
}
static void start_renderer(Renderer* renderer) {
    if (stopping || renderer->retired)
        return;
    g_autoptr(GSubprocessLauncher) launcher = g_subprocess_launcher_new(G_SUBPROCESS_FLAGS_NONE);
    g_autoptr(GError) error = NULL;
    renderer->attempts++;
    renderer->client =
        meta_wayland_client_new_subprocess(meta_wayland_compositor_get_context(frame_compositor),
                                           launcher, (const char* const*)renderer->argv, &error);
    if (!renderer->client) {
        g_warning("Frame renderer %s: %s", renderer->name, error->message);
        return;
    }
    renderer->refs++;
    g_subprocess_wait_async(meta_wayland_client_get_subprocess(renderer->client), NULL,
                            renderer_exited, renderer);
}

static void retire_renderer(Renderer* r) {
    r->retired = TRUE;
    for (GList* l = frames; l; l = l->next) {
        Frame* frame = l->data;
        if (frame->renderer == r) {
            detach_frame(frame);
            frame->renderer = NULL;
        }
    }
    if (r->retry) {
        g_source_remove(r->retry);
        r->retry = 0;
    }
    if (r->client) {
        g_subprocess_force_exit(meta_wayland_client_get_subprocess(r->client));
        meta_wayland_client_destroy(r->client);
        g_clear_object(&r->client);
    }
}

static void prepare_shutdown(MetaWaylandCompositor* compositor, gpointer data) {
    GHashTableIter iter;
    gpointer value;
    stopping = TRUE;
    if (!renderers)
        return;
    for (GList* l = frames; l; l = l->next) {
        Frame* frame = l->data;
        detach_frame(frame);
        frame->renderer = NULL;
    }
    g_hash_table_iter_init(&iter, renderers);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        retire_renderer(value);
    }
    g_clear_pointer(&renderers, g_hash_table_unref);
}

/**
 * meta_gnoblin_frame_renderers_configure:
 * @services: renderer names mapped to argv arrays
 * @restart: restart unchanged services too
 * @error: return location for validation errors
 *
 * Reconcile renderer services without changing client geometry or session
 * state. Returns: whether the complete registry was accepted
 */
gboolean meta_gnoblin_frame_renderers_configure(GVariant* services, gboolean restart,
                                                GError** error) {
    g_autoptr(GHashTable) next =
        g_hash_table_new_full(g_str_hash, g_str_equal, NULL, renderer_unref);
    if (!renderers || stopping)
        return TRUE;
    if (services && !g_variant_is_of_type(services, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "frame-renderers must be a dictionary");
        return FALSE;
    }
    if (services) {
        GVariantIter iter;
        const char* name;
        GVariant* value;
        g_variant_iter_init(&iter, services);
        while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
            GPtrArray* argv = g_ptr_array_new_with_free_func(g_free);
            gboolean valid = g_variant_is_container(value) && g_variant_n_children(value) > 0 &&
                             g_variant_n_children(value) <= 32;
            if (valid)
                for (gsize i = 0; i < g_variant_n_children(value); i++) {
                    GVariant* arg = g_variant_get_child_value(value, i);
                    if (g_variant_is_of_type(arg, G_VARIANT_TYPE_VARIANT)) {
                        GVariant* unwrapped = g_variant_get_variant(arg);
                        g_variant_unref(arg);
                        arg = unwrapped;
                    }
                    if (!g_variant_is_of_type(arg, G_VARIANT_TYPE_STRING))
                        valid = FALSE;
                    else
                        g_ptr_array_add(argv, g_variant_dup_string(arg, NULL));
                    g_variant_unref(arg);
                }
            if (valid && strcmp(name, "native") && ((char*)argv->pdata[0])[0] == '/') {
                Renderer* r = g_new0(Renderer, 1);
                r->refs = 1;
                r->name = g_strdup(name);
                g_ptr_array_add(argv, NULL);
                r->argv = (char**)g_ptr_array_free(argv, FALSE);
                g_hash_table_insert(next, r->name, r);
            } else {
                g_ptr_array_unref(argv);
                g_variant_unref(value);
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Invalid frame renderer: %s", name);
                return FALSE;
            }
            g_variant_unref(value);
        }
    }
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, renderers);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        Renderer* old = value;
        Renderer* replacement = g_hash_table_lookup(next, old->name);
        if (!restart && replacement &&
            g_strv_equal((const char* const*)old->argv, (const char* const*)replacement->argv)) {
            old->refs++;
            g_hash_table_replace(next, old->name, old);
        } else
            retire_renderer(old);
    }
    g_hash_table_unref(renderers);
    renderers = g_steal_pointer(&next);
    for (GList* l = frames; l; l = l->next)
        update_frame(l->data);
    return TRUE;
}

void meta_gnoblin_frame_renderer_init(MetaWaylandCompositor* compositor) {
    frame_compositor = compositor;
    stopping = FALSE;
    g_signal_connect(compositor, "prepare-shutdown", G_CALLBACK(prepare_shutdown), NULL);
    if (!gnoblin_config_protocol_enabled("window-frame-renderer"))
        return;
    renderers = g_hash_table_new_full(g_str_hash, g_str_equal, NULL, renderer_unref);
    g_autofree char* path = gnoblin_config_path();
    g_autoptr(GVariant) document = gnoblin_config_load_document(path, NULL, NULL, NULL);
    g_autoptr(GVariant) services =
        document ? g_variant_lookup_value(document, "frame-renderers", G_VARIANT_TYPE_VARDICT)
                 : NULL;
    meta_gnoblin_frame_renderers_configure(services, FALSE, NULL);
    struct wl_global* global =
        wl_global_create(compositor->wayland_display, &gnoblin_window_frame_manager_v1_interface, 1,
                         NULL, bind_manager);
    if (global)
        meta_wayland_filter_manager_add_global(
            meta_wayland_compositor_get_filter_manager(compositor), global, filter, NULL);
}
