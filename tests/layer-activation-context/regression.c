/* Production activation code, real GLib objects/hash tables and Wayland
 * resources. Display/seat/window boundaries are bounded substitutes. */
#include <gio/gio.h>
#include <stdint.h>
#include <sys/socket.h>
#include <unistd.h>
#include <wayland-server.h>
#include "xdg-activation-v1-server-protocol.h"
#include "tablet-v2-server-protocol.h"

#define META_XDG_ACTIVATION_V1_VERSION 1
#define G_DESKTOP_FOCUS_NEW_WINDOWS_STRICT 0
#define G_DESKTOP_FOCUS_NEW_WINDOWS_SMART 1
#define META_CLIENT_TYPE_APPLICATION 0

typedef struct _MetaWaylandActivation MetaWaylandActivation;
typedef struct _MetaWaylandCompositor MetaWaylandCompositor;
typedef struct _MetaWaylandSeat MetaWaylandSeat;
typedef struct _MetaDisplay MetaDisplay;
typedef MetaDisplay MetaContext;
typedef enum {
    META_WAYLAND_SESSION_LOCK_UNLOCKED,
    LOCK_COVERING,
    LOCK_LOCKED,
    LOCK_FAILSAFE
} MetaWaylandSessionLockState;
typedef void (*MetaWaylandSessionLockStateChangedFunc)(MetaWaylandCompositor*,
                                                       MetaWaylandSessionLockState, gpointer);
typedef enum {
    CLUTTER_BUTTON_PRESS,
    CLUTTER_BUTTON_RELEASE,
    CLUTTER_KEY_PRESS,
    CLUTTER_KEY_RELEASE,
    CLUTTER_TOUCH_BEGIN,
    CLUTTER_SCROLL,
    CLUTTER_MOTION
} ClutterEventType;
#define CLUTTER_EVENT_FLAG_REPEATED 1
#define CLUTTER_EVENT_FLAG_SYNTHETIC 2
#define CLUTTER_EVENT_FLAG_INPUT_METHOD 4
typedef struct {
    ClutterEventType type;
    guint32 time;
    guint flags;
    guint code;
} ClutterEvent;
typedef int ClutterEventSequence;
static ClutterEventSequence* clutter_event_get_event_sequence(const ClutterEvent* event) {
    return (void*)event;
}
static ClutterEventType clutter_event_type(const ClutterEvent* event) {
    return event->type;
}
static guint32 clutter_event_get_time(const ClutterEvent* event) {
    return event->time;
}
static guint clutter_event_get_event_code(const ClutterEvent* event) {
    return event->code;
}
static guint clutter_event_get_flags(const ClutterEvent* event) {
    return event->flags;
}
static gint64 clock_us = 1000000;
#define g_get_monotonic_time() clock_us

typedef struct {
    GObject parent;
    gboolean mapped, unmanaging;
    guint activations, attention, id;
    guint32 timestamp;
} MetaWindow;
typedef GObjectClass MetaWindowClass;
G_DEFINE_TYPE(MetaWindow, meta_window, G_TYPE_OBJECT)
static void meta_window_class_init(MetaWindowClass* klass) {
    g_signal_new("unmanaged", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
                 G_TYPE_NONE, 0);
}
static void meta_window_init(MetaWindow* window) {
    window->mapped = TRUE;
    window->id = 1;
}

typedef struct {
    GObject parent;
    struct wl_resource* resource;
    gboolean layer;
    MetaWindow* window;
    struct {
        GPtrArray* subsurfaces;
    } applied_state;
} MetaWaylandSurface;
#define META_WAYLAND_SURFACE_FOREACH_SUBSURFACE(state, child)                                      \
    for (guint i = 0; (state)->subsurfaces && i < (state)->subsurfaces->len &&                     \
                      ((child = g_ptr_array_index((state)->subsurfaces, i)), TRUE);                \
         i++)
typedef GObjectClass MetaWaylandSurfaceClass;
G_DEFINE_TYPE(MetaWaylandSurface, meta_wayland_surface, G_TYPE_OBJECT)
static void meta_wayland_surface_class_init(MetaWaylandSurfaceClass* klass) {}
static void meta_wayland_surface_init(MetaWaylandSurface* surface) {
    surface->layer = TRUE;
}
typedef struct {
    MetaWaylandSeat* seat;
} MetaWaylandInputDevice;
#define META_WAYLAND_INPUT_DEVICE(device) ((MetaWaylandInputDevice*)device)
typedef struct {
    MetaWaylandInputDevice device;
    MetaWaylandSurface* focus_surface;
    guint32 focus_serial, last_key_up_serial, last_key_up;
    GHashTable* key_down_serials;
    struct wl_list focus_resource_list;
} MetaWaylandKeyboard;
typedef struct {
    struct wl_list pointer_resources;
} PointerClient;
typedef void ClutterSprite;
typedef struct {
    MetaWaylandInputDevice device;
    MetaWaylandSurface* focus_surface;
    PointerClient* focus_client;
    guint32 click_serial, grab_serial;
    guint button_count;
    ClutterSprite* sprite;
    float grab_x, grab_y;
} MetaWaylandPointer;
typedef struct {
    MetaWaylandSurface* surface;
    struct wl_list resource_list;
} TouchSurface;
typedef struct {
    TouchSurface* focus_surface;
    guint32 slot_serial;
    int slot;
    double x, y;
    gboolean begin_delivered;
} MetaWaylandTouchInfo;
typedef struct {
    MetaWaylandInputDevice device;
    MetaWaylandTouchInfo* info;
} MetaWaylandTouch;
static MetaWaylandTouchInfo* touch_get_info(MetaWaylandTouch* touch, ClutterEventSequence* sequence,
                                            gboolean create) {
    return touch->info;
}
typedef struct {
    struct wl_display* wl_display;
} TabletManager;
typedef struct {
    TabletManager* manager;
    MetaWaylandSeat* seat;
} TabletSeat;
typedef struct {
    TabletSeat* seat;
    MetaWaylandSurface* focus_surface;
    struct wl_list focus_resource_list;
    guint32 down_serial, button_serial;
} MetaWaylandTabletTool;
static guint32 next_serial;
static MetaWaylandSeat* meta_wayland_input_device_get_seat(MetaWaylandInputDevice* device) {
    return device->seat;
}
static guint32 meta_wayland_input_device_next_serial(MetaWaylandInputDevice* device) {
    return next_serial++;
}
static void meta_wayland_pointer_broadcast_frame(MetaWaylandPointer* pointer) {}
static void sync_focus_surface(MetaWaylandPointer* pointer) {}

struct _MetaWaylandSeat {
    MetaWaylandCompositor* compositor;
    MetaWaylandKeyboard* keyboard;
    MetaWaylandPointer* pointer;
    MetaWaylandSurface* other_grab_surface;
    guint32 other_grab_serial;
    gboolean other_device_grab;
};
struct _MetaDisplay {
    guint32 now, last_user_time;
    GHashTable* startup_notification;
    guint denials;
    char* reason;
};
struct _MetaWaylandCompositor {
    MetaDisplay* display;
    MetaWaylandSeat* seat;
    MetaWaylandActivation* activation;
    struct wl_display* wayland_display;
    gboolean locked;
    MetaWaylandSessionLockStateChangedFunc lock_callback;
    gpointer lock_data;
    GDestroyNotify lock_destroy;
};
static gboolean gnoblin = TRUE;
static int policy = G_DESKTOP_FOCUS_NEW_WINDOWS_STRICT;
static gboolean meta_is_gnoblin_session(void) {
    return gnoblin;
}
static int meta_prefs_get_focus_new_windows(void) {
    return policy;
}
static MetaContext* meta_wayland_compositor_get_context(MetaWaylandCompositor* c) {
    return c->display;
}
static MetaDisplay* meta_context_get_display(MetaContext* c) {
    return c;
}
static guint32 meta_display_get_current_time(MetaDisplay* d) {
    return d->now;
}
static guint32 meta_display_get_current_time_roundtrip(MetaDisplay* d) {
    return d->now;
}
static guint32 meta_display_get_last_user_time(MetaDisplay* d) {
    return d->last_user_time;
}
static MetaWindow* meta_wayland_surface_get_window(MetaWaylandSurface* s) {
    return s->window;
}
static gboolean meta_wayland_surface_is_layer_shell(MetaWaylandSurface* s) {
    return s->layer;
}
static MetaWaylandCompositor* meta_wayland_seat_get_compositor(MetaWaylandSeat* s) {
    return s->compositor;
}
static gboolean meta_wayland_session_lock_is_active(MetaWaylandCompositor* c) {
    return c->locked;
}
gboolean meta_wayland_keyboard_can_popup(MetaWaylandKeyboard* k, guint32 serial);
gboolean meta_wayland_keyboard_can_grab_surface(MetaWaylandKeyboard* k, MetaWaylandSurface* s,
                                                guint32 serial);
gboolean meta_wayland_pointer_get_grab_info(MetaWaylandPointer* pointer,
                                            MetaWaylandSurface* surface, guint32 serial,
                                            gboolean pressed, ClutterSprite** sprite, float* x,
                                            float* y);
static gboolean meta_wayland_seat_get_grab_info(MetaWaylandSeat* s, MetaWaylandSurface* surface,
                                                guint32 serial, gboolean pressed,
                                                ClutterSprite** sprite, float* x, float* y) {
    return meta_wayland_pointer_get_grab_info(s->pointer, surface, serial, pressed, sprite, x, y) ||
           (s->other_device_grab && s->other_grab_surface == surface &&
            s->other_grab_serial == serial);
}
static gulong meta_wayland_session_lock_add_state_changed_callback(
    MetaWaylandCompositor* c, MetaWaylandSessionLockStateChangedFunc callback, gpointer data,
    GDestroyNotify destroy) {
    c->lock_callback = callback;
    c->lock_data = data;
    c->lock_destroy = destroy;
    return 1;
}
static void meta_wayland_session_lock_remove_state_changed_callback(MetaWaylandCompositor* c,
                                                                    gulong id) {
    if (c->lock_destroy)
        c->lock_destroy(c->lock_data);
    c->lock_callback = NULL;
    c->lock_data = NULL;
    c->lock_destroy = NULL;
}
static void set_lock(MetaWaylandCompositor* c, MetaWaylandSessionLockState state) {
    c->locked = state != META_WAYLAND_SESSION_LOCK_UNLOCKED;
    if (c->lock_callback)
        c->lock_callback(c, state, c->lock_data);
}
static guint meta_window_get_stable_sequence(MetaWindow* w) {
    return w->id;
}
static void meta_display_dispatch_gnoblin_native_event(MetaDisplay* d, const char* name,
                                                       GVariant* payload) {
    d->denials++;
    g_free(d->reason);
    g_variant_lookup(payload, "reason", "s", &d->reason);
}
static void (*activation_effect)(void);
static void meta_window_activate_full(MetaWindow* w, guint32 timestamp, int type, void* workspace) {
    w->activations++;
    w->timestamp = timestamp;
    if (activation_effect)
        activation_effect();
}
static void meta_window_set_demands_attention(MetaWindow* w) {
    w->attention++;
}
static void meta_window_change_workspace_by_index(MetaWindow* w, int index, gboolean create) {}

typedef struct {
    GObject parent;
    MetaDisplay* display;
    char* id;
    guint32 timestamp;
} MetaStartupSequence;
typedef GObjectClass MetaStartupSequenceClass;
G_DEFINE_TYPE(MetaStartupSequence, meta_startup_sequence, G_TYPE_OBJECT)
#define META_TYPE_STARTUP_SEQUENCE (meta_startup_sequence_get_type())
static void sequence_set_property(GObject* object, guint prop, const GValue* value,
                                  GParamSpec* pspec) {
    MetaStartupSequence* s = (MetaStartupSequence*)object;
    if (prop == 1)
        s->display = g_value_get_pointer(value);
    if (prop == 2)
        s->id = g_value_dup_string(value);
    if (prop == 4)
        s->timestamp = (guint32)g_value_get_uint64(value);
}
static void sequence_finalize(GObject* object) {
    g_free(((MetaStartupSequence*)object)->id);
    G_OBJECT_CLASS(meta_startup_sequence_parent_class)->finalize(object);
}
static void meta_startup_sequence_class_init(MetaStartupSequenceClass* klass) {
    klass->set_property = sequence_set_property;
    klass->finalize = sequence_finalize;
    g_object_class_install_property(klass, 1,
                                    g_param_spec_pointer("display", NULL, NULL, G_PARAM_WRITABLE));
    g_object_class_install_property(klass, 2,
                                    g_param_spec_string("id", NULL, NULL, NULL, G_PARAM_WRITABLE));
    g_object_class_install_property(
        klass, 3, g_param_spec_string("application-id", NULL, NULL, NULL, G_PARAM_WRITABLE));
    g_object_class_install_property(
        klass, 4,
        g_param_spec_uint64("timestamp", NULL, NULL, 0, G_MAXUINT64, 0, G_PARAM_WRITABLE));
    g_signal_new("complete", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
                 G_TYPE_NONE, 0);
    g_signal_new("timeout", G_TYPE_FROM_CLASS(klass), G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
                 G_TYPE_NONE, 0);
}
static void meta_startup_sequence_init(MetaStartupSequence* s) {}
static MetaDisplay* meta_startup_sequence_get_display(MetaStartupSequence* s) {
    return s->display;
}
static guint32 meta_startup_sequence_get_timestamp(MetaStartupSequence* s) {
    return s->timestamp;
}
static int meta_startup_sequence_get_workspace(MetaStartupSequence* s) {
    return -1;
}
static void meta_startup_notification_add_sequence(GHashTable* table, MetaStartupSequence* s) {
    g_hash_table_insert(table, g_strdup(s->id), g_object_ref(s));
}
static void meta_startup_notification_remove_sequence(GHashTable* table, MetaStartupSequence* s) {
    g_hash_table_remove(table, s->id);
}
static MetaStartupSequence* meta_startup_notification_lookup_sequence(GHashTable* table,
                                                                      const char* id) {
    return g_hash_table_lookup(table, id);
}
static void meta_startup_sequence_complete(MetaStartupSequence* s) {
    g_object_ref(s);
    g_signal_emit_by_name(s, "complete");
    g_object_unref(s);
}

#include "activation.inc"
#include "input-delivery.inc"

typedef struct {
    MetaDisplay display;
    MetaWaylandCompositor compositor;
    MetaWaylandSeat seat, foreign_seat;
    MetaWaylandKeyboard keyboard;
    MetaWaylandPointer pointer;
    PointerClient pointer_client;
    struct wl_client *client, *other_client;
    struct wl_resource *activation_resource, *seat_resource;
    MetaWaylandSurface *source, *other_surface;
    MetaWindow* target;
    GPtrArray* token_resources;
    int sockets[2];
} Fixture;
static MetaWaylandSurface* new_surface(Fixture* f, struct wl_client* client) {
    MetaWaylandSurface* s = g_object_new(meta_wayland_surface_get_type(), NULL);
    s->resource = wl_resource_create(client, &wl_surface_interface, 1, 0);
    wl_resource_set_user_data(s->resource, s);
    return s;
}
static void setup(Fixture* f, gconstpointer data) {
    int fd[2];
    activation_effect = NULL;
    gnoblin = TRUE;
    policy = G_DESKTOP_FOCUS_NEW_WINDOWS_STRICT;
    clock_us = 1000000;
    f->display.now = 100;
    f->display.last_user_time = 100;
    f->display.startup_notification =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_object_unref);
    f->compositor.display = &f->display;
    f->compositor.seat = &f->seat;
    f->compositor.wayland_display = wl_display_create();
    f->seat.compositor = &f->compositor;
    f->seat.keyboard = &f->keyboard;
    f->seat.pointer = &f->pointer;
    f->foreign_seat = f->seat;
    f->keyboard.key_down_serials = g_hash_table_new(NULL, NULL);
    f->keyboard.device.seat = &f->seat;
    f->pointer.device.seat = &f->seat;
    wl_list_init(&f->keyboard.focus_resource_list);
    wl_list_init(&f->pointer_client.pointer_resources);
    f->pointer.focus_client = &f->pointer_client;
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_STREAM, 0, fd), ==, 0);
    f->client = wl_client_create(f->compositor.wayland_display, fd[0]);
    f->sockets[0] = fd[1];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_STREAM, 0, fd), ==, 0);
    f->other_client = wl_client_create(f->compositor.wayland_display, fd[0]);
    f->sockets[1] = fd[1];
    struct wl_resource* keyboard_resource =
        wl_resource_create(f->client, &wl_keyboard_interface, 1, 0);
    struct wl_resource* pointer_resource =
        wl_resource_create(f->client, &wl_pointer_interface, 1, 0);
    wl_list_insert(&f->keyboard.focus_resource_list, wl_resource_get_link(keyboard_resource));
    wl_list_insert(&f->pointer_client.pointer_resources, wl_resource_get_link(pointer_resource));
    f->activation_resource = wl_resource_create(f->client, &xdg_activation_v1_interface, 1, 0);
    f->seat_resource = wl_resource_create(f->client, &wl_seat_interface, 1, 0);
    wl_resource_set_user_data(f->seat_resource, &f->seat);
    meta_wayland_activation_init(&f->compositor);
    f->source = new_surface(f, f->client);
    f->other_surface = new_surface(f, f->other_client);
    f->target = g_object_new(meta_window_get_type(), NULL);
    f->token_resources = g_ptr_array_new();
}
static void teardown(Fixture* f, gconstpointer data) {
    for (guint i = 0; i < f->token_resources->len; i++)
        wl_resource_destroy(g_ptr_array_index(f->token_resources, i));
    g_ptr_array_unref(f->token_resources);
    /* Match real compositor order: lock controller goes away first. */
    meta_wayland_session_lock_remove_state_changed_callback(&f->compositor, 1);
    meta_wayland_activation_finalize(&f->compositor);
    wl_display_destroy_clients(f->compositor.wayland_display);
    wl_display_destroy(f->compositor.wayland_display);
    close(f->sockets[0]);
    close(f->sockets[1]);
    g_object_unref(f->source);
    g_object_unref(f->other_surface);
    g_object_unref(f->target);
    g_hash_table_unref(f->display.startup_notification);
    g_hash_table_unref(f->keyboard.key_down_serials);
    g_free(f->display.reason);
}
static void input(Fixture* f, MetaWaylandSurface* surface, guint32 serial, ClutterEventType type) {
    ClutterEvent event = {type, f->display.now, 0, 1};
    next_serial = serial;
#if PATCHED
    meta_wayland_activation_update_input(&f->compositor, &event);
#endif
    if (type == CLUTTER_KEY_PRESS || type == CLUTTER_KEY_RELEASE) {
        f->keyboard.focus_surface = surface;
        meta_wayland_keyboard_broadcast_key(&f->keyboard, &event);
    } else {
        f->pointer.focus_surface = surface;
        f->pointer.button_count = type == CLUTTER_BUTTON_PRESS;
        meta_wayland_pointer_send_button(&f->pointer, &event);
        /* handle_button_event saves the serial of the initial press for grabs. */
        if (type == CLUTTER_BUTTON_PRESS)
            f->pointer.grab_serial = serial;
    }
    (void)event;
}
static struct wl_resource* request(Fixture* f) {
    meta_wayland_activation_token_create_new_resource(f->compositor.activation, f->client,
                                                      f->activation_resource, 0);
    struct wl_resource* r = wl_resource_from_link(f->compositor.activation->token_list.next);
    g_ptr_array_add(f->token_resources, r);
    return r;
}
static char* commit(Fixture* f, MetaWaylandSurface* surface, guint32 serial,
                    struct wl_resource* seat) {
    struct wl_resource* r = request(f);
    if (surface)
        token_set_surface(f->client, r, surface->resource);
    if (seat)
        token_set_serial(f->client, r, serial, seat);
    token_commit(f->client, r);
    return g_strdup(((MetaXdgActivationToken*)wl_resource_get_user_data(r))->token);
}
static void dismiss(Fixture* f) {
    f->keyboard.focus_surface = NULL;
    f->pointer.focus_surface = NULL;
    f->seat.other_grab_surface = NULL;
}
static void activate(Fixture* f, const char* token, gboolean allowed) {
    guint before = f->target->activations;
    g_assert_true(maybe_activate(f->compositor.activation, f->target, token));
    g_assert_cmpuint(f->target->activations, ==, before + allowed);
}
static void lifecycle(Fixture* f, gconstpointer data) {
    const char* mode = data;
    gboolean key = g_str_has_prefix(mode, "key");
    input(f, f->source, 10, key ? CLUTTER_KEY_PRESS : CLUTTER_BUTTON_PRESS);
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    if (strstr(mode, "dismiss") || strstr(mode, "destroy"))
        dismiss(f);
    if (strstr(mode, "destroy")) {
        wl_resource_destroy(f->source->resource);
        f->source->resource = NULL;
    }
    activate(f, token, TRUE);
    g_assert_cmpuint(f->target->timestamp, ==, 100);
    activate(f, token, FALSE);
}
static void invalid(Fixture* f, gconstpointer data) {
    const char* mode = data;
    input(f, f->source, 10, CLUTTER_BUTTON_PRESS);
    MetaWaylandSurface* source = f->source;
    struct wl_resource* seat = f->seat_resource;
    guint32 serial = 10;
    if (!strcmp(mode, "surface"))
        source = NULL;
    if (!strcmp(mode, "seat"))
        seat = NULL;
    if (!strcmp(mode, "serial"))
        serial = 999;
    if (!strcmp(mode, "foreign-seat"))
        wl_resource_set_user_data(seat, &f->foreign_seat);
    if (!strcmp(mode, "foreign-source"))
        source = f->other_surface;
    if (!strcmp(mode, "wrong-recipient")) {
        /* Model the permissive old grab helper after a surface switch. */
        f->pointer.focus_surface = f->other_surface;
        source = f->other_surface;
        wl_resource_destroy(source->resource);
        source->resource = wl_resource_create(f->client, &wl_surface_interface, 1, 0);
        wl_resource_set_user_data(source->resource, source);
    }
    if (!strcmp(mode, "focus-only")) {
        f->pointer.focus_surface = NULL;
        f->seat.other_grab_surface = NULL;
        f->keyboard.focus_surface = f->source;
        f->keyboard.focus_serial = serial = 11;
    }
    if (!strcmp(mode, "unfocused"))
        dismiss(f);
    if (!strcmp(mode, "locked"))
        set_lock(&f->compositor, LOCK_COVERING);
    if (!strcmp(mode, "old-input"))
        clock_us += 16000000;
    g_autofree char* token = commit(f, source, serial, seat);
    if (!strcmp(mode, "locked"))
        set_lock(&f->compositor, META_WAYLAND_SESSION_LOCK_UNLOCKED);
    activate(f, token, FALSE);
}
static void later(Fixture* f, gconstpointer data) {
    const char* mode = data;
    input(f, f->source, 10, CLUTTER_BUTTON_PRESS);
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    if (!strcmp(mode, "press"))
        input(f, f->source, 11, CLUTTER_BUTTON_PRESS);
    if (!strcmp(mode, "key"))
        input(f, f->source, 11, CLUTTER_KEY_PRESS);
    if (!strcmp(mode, "release"))
        input(f, f->source, 11, CLUTTER_BUTTON_RELEASE);
    if (!strcmp(mode, "user-time"))
        f->display.last_user_time++;
    if (!strcmp(mode, "expiry"))
        clock_us += 16000000;
    if (!strcmp(mode, "shortcut")) {
        ClutterEvent event = {CLUTTER_KEY_PRESS, 100, 0, 1};
#if PATCHED
        meta_wayland_activation_update_input(&f->compositor, &event);
#endif
        (void)event;
    }
    if (!strcmp(mode, "lock") || !strcmp(mode, "unlock")) {
        set_lock(&f->compositor, LOCK_COVERING);
        if (!strcmp(mode, "unlock"))
            set_lock(&f->compositor, META_WAYLAND_SESSION_LOCK_UNLOCKED);
    }
    if (!strcmp(mode, "timeout")) {
        MetaXdgActivationToken* t = g_hash_table_lookup(f->compositor.activation->tokens, token);
        MetaStartupSequence* s = g_object_ref(t->sequence);
        meta_startup_notification_remove_sequence(f->display.startup_notification, s);
        g_signal_emit_by_name(s, "timeout");
        g_object_unref(s);
    }
    activate(f, token, !strcmp(mode, "release"));
    activate(f, token, FALSE);
}
static void typed(Fixture* f, gconstpointer data) {
    input(f, f->source, 10, CLUTTER_BUTTON_PRESS);
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    g_assert_false(meta_wayland_activation_focus_window_with_token(&f->compositor, f->target, token,
                                                                   getpid() + 1));
    dismiss(f);
    g_assert_true(meta_wayland_activation_focus_window_with_token(&f->compositor, f->target, token,
                                                                  getpid()));
    g_assert_false(meta_wayland_activation_focus_window_with_token(&f->compositor, f->target, token,
                                                                   getpid()));
}
static void upstream(Fixture* f, gconstpointer data) {
    gnoblin = FALSE;
    input(f, f->source, 10, CLUTTER_BUTTON_PRESS);
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    dismiss(f);
    activate(f, token, FALSE);
    input(f, f->source, 11, CLUTTER_BUTTON_PRESS);
    g_autofree char* held = commit(f, f->source, 11, f->seat_resource);
    activate(f, held, TRUE);
}
static void timestamp(Fixture* f, gconstpointer data) {
    f->display.now = G_MAXUINT32 - 5;
    f->display.last_user_time = G_MAXUINT32 - 6;
    input(f, f->source, 10, CLUTTER_BUTTON_PRESS);
    f->display.now = 2;
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    dismiss(f);
    activate(f, token, TRUE);
    g_assert_cmpuint(f->target->timestamp, ==, G_MAXUINT32 - 5);
}
static void queued(Fixture* f, gconstpointer data) {
    input(f, f->source, 10, CLUTTER_BUTTON_PRESS);
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    f->target->mapped = FALSE;
    g_assert_false(maybe_activate(f->compositor.activation, f->target, token));
    add_pending_activate(f->compositor.activation, f->target, token);
    dismiss(f);
    f->target->mapped = TRUE;
    complete_pending_activate(f->compositor.activation, f->target);
    g_assert_cmpuint(f->target->activations, ==, 1);
}
static Fixture* reentrant_fixture;
static const char* reentrant_token;
static void replay_during_focus(void) {
    activation_effect = NULL;
    activate(reentrant_fixture, reentrant_token, FALSE);
}
static void boundaries(Fixture* f, gconstpointer data) {
    const char* mode = data;
    input(f, f->source, 10, CLUTTER_BUTTON_PRESS);
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    struct wl_resource* r = g_ptr_array_index(f->token_resources, 0);
    if (!strcmp(mode, "destroy-token-object")) {
        g_ptr_array_remove_index(f->token_resources, 0);
        wl_resource_destroy(r);
        dismiss(f);
        activate(f, token, TRUE);
    } else if (!strcmp(mode, "transfer-client")) {
        f->other_surface->window = f->target;
        wl_resource_set_user_data(f->activation_resource, f->compositor.activation);
        dismiss(f);
        activation_activate(f->other_client, f->activation_resource, token,
                            f->other_surface->resource);
        g_assert_cmpuint(f->target->activations, ==, 1);
    } else if (!strcmp(mode, "reentrant-replay")) {
        reentrant_fixture = f;
        reentrant_token = token;
        activation_effect = replay_during_focus;
        activate(f, token, TRUE);
    } else if (!strcmp(mode, "standard-then-typed")) {
        activate(f, token, TRUE);
        g_assert_false(meta_wayland_activation_focus_window_with_token(&f->compositor, f->target,
                                                                       token, getpid()));
    } else if (!strcmp(mode, "typed-then-standard")) {
        g_assert_true(meta_wayland_activation_focus_window_with_token(&f->compositor, f->target,
                                                                      token, getpid()));
        activate(f, token, FALSE);
    } else if (!strcmp(mode, "typed-later-input")) {
        input(f, f->source, 11, CLUTTER_KEY_PRESS);
        g_assert_false(meta_wayland_activation_focus_window_with_token(&f->compositor, f->target,
                                                                       token, getpid()));
        activate(f, token, FALSE);
    } else if (!strcmp(mode, "post-consumption-request")) {
        activate(f, token, TRUE);
#if PATCHED
        g_assert_null(wl_resource_get_user_data(r));
        token_commit(f->client, r);
        token_set_serial(f->client, r, 10, f->seat_resource);
        token_set_surface(f->client, r, f->source->resource);
        token_set_app_id(f->client, r, "mutated");
#endif
    } else if (!strcmp(mode, "mutation-after-commit")) {
        token_set_serial(f->client, r, 999, f->seat_resource);
        MetaXdgActivationToken* t = wl_resource_get_user_data(r);
        g_assert_cmpuint(t->serial, ==, 10);
    }
}
static void no_delivery(Fixture* f, gconstpointer data) {
    /* A seat event/serial without a recipient must not create a grant. */
    struct wl_resource* r = wl_resource_from_link(f->pointer_client.pointer_resources.next);
    wl_list_remove(wl_resource_get_link(r));
    input(f, f->source, 10, CLUTTER_BUTTON_PRESS);
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    activate(f, token, FALSE);
}
static void nonlayer(Fixture* f, gconstpointer data) {
    f->source->layer = FALSE;
    f->keyboard.focus_surface = f->source;
    f->keyboard.focus_serial = 10;
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    activate(f, token, TRUE);
}
static void allow_policy(Fixture* f, gconstpointer data) {
    policy = G_DESKTOP_FOCUS_NEW_WINDOWS_SMART;
    g_autofree char* token = commit(f, f->source, 10, f->seat_resource);
    activate(f, token, TRUE);
    activate(f, "unknown-token", TRUE);
}
static void resource_lifetime(Fixture* f, gconstpointer data) {
    struct wl_resource* r = request(f);
    token_set_surface(f->client, r, f->source->resource);
    token_set_surface(f->client, r, f->other_surface->resource);
    /* Replacing or destroying an uncommitted request must detach listeners. */
    wl_resource_destroy(f->source->resource);
    f->source->resource = NULL;
    g_ptr_array_remove_index(f->token_resources, 0);
    wl_resource_destroy(r);
    wl_resource_destroy(f->other_surface->resource);
    f->other_surface->resource = NULL;
}
static void other_devices(Fixture* f, gconstpointer data) {
    const char* mode = data;
    guint32 serial;
    ClutterEvent event = {CLUTTER_TOUCH_BEGIN, 100, 0, 1};
#if PATCHED
    meta_wayland_activation_update_input(&f->compositor, &event);
#endif
    if (!strcmp(mode, "touch")) {
        TouchSurface focus = {.surface = f->source};
        wl_list_init(&focus.resource_list);
        struct wl_resource* r = wl_resource_create(f->client, &wl_touch_interface, 1, 0);
        wl_list_insert(&focus.resource_list, wl_resource_get_link(r));
        MetaWaylandTouchInfo info = {.focus_surface = &focus, .slot_serial = 10};
        MetaWaylandTouch touch = {.device = {&f->seat}, .info = &info};
        handle_touch_begin(&touch, &event);
        serial = info.slot_serial;
        wl_list_remove(wl_resource_get_link(r));
    } else {
        TabletManager manager = {f->compositor.wayland_display};
        TabletSeat seat = {&manager, &f->seat};
        MetaWaylandTabletTool tool = {.seat = &seat, .focus_surface = f->source};
        wl_list_init(&tool.focus_resource_list);
        struct wl_resource* r = wl_resource_create(f->client, &zwp_tablet_tool_v2_interface, 1, 0);
        wl_list_insert(&tool.focus_resource_list, wl_resource_get_link(r));
        event.type = CLUTTER_BUTTON_PRESS;
        if (!strcmp(mode, "tablet-down")) {
            broadcast_down(&tool, &event);
            serial = tool.down_serial;
        } else {
            broadcast_button(&tool, &event);
            serial = tool.button_serial;
        }
        wl_list_remove(wl_resource_get_link(r));
    }
    /* The device grab lookup is the substituted boundary, as for pointer. */
    f->seat.other_device_grab = TRUE;
    f->seat.other_grab_surface = f->source;
    f->seat.other_grab_serial = serial;
    g_autofree char* token = commit(f, f->source, serial, f->seat_resource);
    dismiss(f);
    activate(f, token, TRUE);
}
static void release_before_commit(Fixture* f, gconstpointer data) {
    const char* mode = data;
    gboolean keyboard = !strcmp(mode, "keyboard");
    guint32 press_time = 48143282;
    f->display.now = press_time;
    f->display.last_user_time = press_time;
    input(f, f->source, 35, keyboard ? CLUTTER_KEY_PRESS : CLUTTER_BUTTON_PRESS);
    f->display.now = 48143403;
    if (!strcmp(mode, "expired"))
        clock_us += 16000000;
    input(f, f->source, 36, keyboard ? CLUTTER_KEY_RELEASE : CLUTTER_BUTTON_RELEASE);
    if (!keyboard)
        g_assert_true(meta_wayland_pointer_get_grab_info(&f->pointer, f->source, 35, FALSE, NULL,
                                                         NULL, NULL));
    else
        g_assert_true(meta_wayland_keyboard_can_grab_surface(&f->keyboard, f->source, 35));
    if (!strcmp(mode, "later-press"))
        input(f, f->source, 37, CLUTTER_BUTTON_PRESS);
    /* Qt lastInputSerial retains 35 while the seat's most recent serial is 36. */
    g_autofree char* token = commit(f, f->source, 35, f->seat_resource);
    gboolean allowed = strcmp(mode, "expired") && strcmp(mode, "later-press");
#if PATCHED
    MetaXdgActivationToken* t = g_hash_table_lookup(f->compositor.activation->tokens, token);
    g_assert_cmpint(t->gnoblin_authorized, ==, allowed);
#endif
    dismiss(f);
    activate(f, token, allowed);
    if (allowed)
        g_assert_cmpuint(f->target->timestamp, ==, press_time);
    activate(f, token, FALSE);
}
int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    const char* valid[] = {"held", "dismiss", "destroy", "key-dismiss"};
    for (guint i = 0; i < G_N_ELEMENTS(valid); i++) {
        g_autofree char* path = g_strdup_printf("/lifecycle/%s", valid[i]);
        g_test_add(path, Fixture, valid[i], setup, lifecycle, teardown);
    }
    const char* bad[] = {
        "surface",         "seat",       "serial",    "foreign-seat", "foreign-source",
        "wrong-recipient", "focus-only", "unfocused", "locked",       "old-input"};
    for (guint i = 0; i < G_N_ELEMENTS(bad); i++) {
        g_autofree char* path = g_strdup_printf("/invalid/%s", bad[i]);
        g_test_add(path, Fixture, bad[i], setup, invalid, teardown);
    }
    const char* after[] = {"press",    "key",  "release", "user-time", "expiry",
                           "shortcut", "lock", "unlock",  "timeout"};
    for (guint i = 0; i < G_N_ELEMENTS(after); i++) {
        g_autofree char* path = g_strdup_printf("/later/%s", after[i]);
        g_test_add(path, Fixture, after[i], setup, later, teardown);
    }
    g_test_add("/typed/peer-and-replay", Fixture, NULL, setup, typed, teardown);
    g_test_add("/compat/non-gnoblin", Fixture, NULL, setup, upstream, teardown);
    g_test_add("/lifecycle/wraparound", Fixture, NULL, setup, timestamp, teardown);
    g_test_add("/lifecycle/queued", Fixture, NULL, setup, queued, teardown);
    const char* edges[] = {"destroy-token-object",     "transfer-client",      "reentrant-replay",
                           "standard-then-typed",      "typed-then-standard",  "typed-later-input",
                           "post-consumption-request", "mutation-after-commit"};
    for (guint i = 0; i < G_N_ELEMENTS(edges); i++) {
        g_autofree char* path = g_strdup_printf("/boundary/%s", edges[i]);
        g_test_add(path, Fixture, edges[i], setup, boundaries, teardown);
    }
    g_test_add("/invalid/no-delivery", Fixture, NULL, setup, no_delivery, teardown);
    g_test_add("/compat/nonlayer-focus-serial", Fixture, NULL, setup, nonlayer, teardown);
    g_test_add("/compat/allow-policy", Fixture, NULL, setup, allow_policy, teardown);
    g_test_add("/boundary/resource-lifetime", Fixture, NULL, setup, resource_lifetime, teardown);
    g_test_add("/delivery/touch", Fixture, "touch", setup, other_devices, teardown);
    g_test_add("/delivery/tablet-down", Fixture, "tablet-down", setup, other_devices, teardown);
    g_test_add("/delivery/tablet-button", Fixture, "tablet-button", setup, other_devices, teardown);
    g_test_add("/gesture/pointer-release-then-commit", Fixture, "pointer", setup,
               release_before_commit, teardown);
    g_test_add("/gesture/keyboard-release-then-commit", Fixture, "keyboard", setup,
               release_before_commit, teardown);
    g_test_add("/gesture/release-does-not-refresh-expiry", Fixture, "expired", setup,
               release_before_commit, teardown);
    g_test_add("/gesture/later-press-before-commit", Fixture, "later-press", setup,
               release_before_commit, teardown);
    return g_test_run();
}
