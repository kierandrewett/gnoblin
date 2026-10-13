/* Production stage/filter/display/pointer routing, with service boundaries stubbed. */
#include "events.h"
#include <stdint.h>

typedef void ClutterStage;
typedef void ClutterInputDevice;
typedef void ClutterSeat;
typedef void ClutterBackend;
typedef void ClutterSprite;
typedef void MetaBackend;
typedef void MetaContext;
typedef void MetaA11yManager;
typedef void MetaCompositor;
typedef void MetaTabletActionMapper;
typedef void MetaWaylandTextInput;
typedef struct {
    gboolean override_redirect;
    void* close_dialog;
} MetaWindow;
typedef struct {
    void* focus_surface;
} MetaWaylandPointer;
typedef struct {
    MetaWaylandPointer* pointer;
} MetaWaylandSeat;
typedef struct {
    MetaWaylandPointer* pointer;
    MetaWaylandSeat* seat;
} MetaWaylandCompositor;
typedef struct {
    GObject parent;
    gboolean native_key_capture_active;
    void (*native_key_capture_handler)(const ClutterEvent*, gpointer);
    gpointer native_key_capture_data;
    MetaWindow* focus_window;
    gpointer pad_action_mapper, tool_action_mapper, current_pad_osd;
    guint32 current_time;
} MetaDisplay;
typedef struct {
    GSList* current_event;
    GList* event_filters;
} ClutterContext;
typedef struct {
    ClutterStage* stage;
    gboolean (*func)(const ClutterEvent*, ClutterActor*, gpointer);
    gpointer user_data;
} ClutterEventFilter;

static MetaDisplay* display;
static ClutterContext context;
static MetaWindow window;
static MetaWaylandPointer pointer;
static MetaWaylandSeat seat = {&pointer};
static MetaWaylandCompositor compositor = {&pointer, &seat};
static gboolean locked, captured, grabbed, replay, consumed, policy_queued, binding;
static guint ui_calls, stage_calls, client_buttons, client_motion, seat_updates, window_presses;
static guint policy_calls, pointer_focus_clears;
static gboolean console_modal;

#define HAVE_WAYLAND 1
#define IS_KEY_EVENT(t) ((t) == CLUTTER_KEY_PRESS || (t) == CLUTTER_KEY_RELEASE)
#define META_CURRENT_TIME 0
#define CLUTTER_CURRENT_TIME 0
#define META_DEBUG_X11 0
#define CLUTTER_INPUT_CAPABILITY_TABLET_TOOL 1
#define META_TABLET_ACTION_MAPPER(p) (p)
#define CLUTTER_ACTOR(p) (p)
#define CLUTTER_STAGE(p) (p)
#define CLUTTER_IS_STAGE(p) ((p) != NULL)
#define CLUTTER_ACTOR_IN_DESTRUCTION(p) FALSE
#define COGL_TRACE_BEGIN_SCOPED(...)
#define COGL_TRACE_DESCRIBE(...)
#define meta_topic(...)

static gboolean event_callback(const ClutterEvent*, ClutterActor*, gpointer);
static gboolean meta_wayland_pointer_handle_event(MetaWaylandPointer*, const ClutterEvent*);
static ClutterStage* stage_from_display(MetaDisplay* d) {
    return &context;
}
static gboolean stage_has_grab(MetaDisplay* d) {
    return grabbed;
}
static gboolean stage_has_key_focus(MetaDisplay* d) {
    return TRUE;
}
static MetaContext* meta_display_get_context(MetaDisplay* d) {
    return &context;
}
static MetaBackend* meta_context_get_backend(MetaContext* c) {
    return &context;
}
static MetaA11yManager* meta_backend_get_a11y_manager(MetaBackend* b) {
    return &context;
}
static MetaCompositor* meta_display_get_compositor(MetaDisplay* d) {
    return &context;
}
static ClutterBackend* meta_backend_get_clutter_backend(MetaBackend* b) {
    return &context;
}
static ClutterSeat* clutter_backend_get_default_seat(ClutterBackend* b) {
    return &context;
}
static MetaWaylandCompositor* meta_context_get_wayland_compositor(MetaContext* c) {
    return &compositor;
}
static MetaWaylandTextInput* meta_wayland_compositor_get_text_input(MetaWaylandCompositor* c) {
    return NULL;
}
static gboolean clutter_stage_gnoblin_input_policy_replay_is_active(ClutterStage* s,
                                                                    const ClutterEvent* e) {
    return replay;
}
static gboolean meta_display_gnoblin_input_policy_allowed(MetaDisplay* d) {
    return !captured && !grabbed && !d->native_key_capture_active;
}
static gboolean meta_wayland_session_lock_is_active(MetaWaylandCompositor* c) {
    return locked;
}
static gboolean meta_wayland_session_lock_filter_event(MetaWaylandCompositor* c,
                                                       const ClutterEvent* e) {
    return locked;
}
static gboolean meta_display_process_captured_input(MetaDisplay* d, const ClutterEvent* e) {
    return captured;
}
static gboolean clutter_stage_gnoblin_input_policy_replay_is_consumed(ClutterStage* s,
                                                                      const ClutterEvent* e) {
    return replay && consumed;
}
static gboolean meta_window_shortcuts_inhibited(MetaWindow* w, ClutterInputDevice* d) {
    return FALSE;
}
static gboolean meta_a11y_manager_notify_clients(MetaA11yManager* a, const ClutterEvent* e) {
    return FALSE;
}
static void meta_a11y_manager_maybe_notify_motion(MetaA11yManager* a) {}
static ClutterInputDevice* clutter_event_get_source_device(const ClutterEvent* e) {
    return &context;
}
static void* clutter_event_get_device_tool(const ClutterEvent* e) {
    return NULL;
}
static void clutter_seat_a11y_update(ClutterSeat* s, const ClutterEvent* e) {}
static void* meta_compositor_get_current_window_drag(MetaCompositor* c) {
    return NULL;
}
static gboolean meta_wayland_text_input_update(MetaWaylandTextInput* t, const ClutterEvent* e) {
    return FALSE;
}
static void meta_display_handle_sticky_mouse_focus_event(MetaDisplay* d, const ClutterEvent* e) {}
static void meta_wayland_pointer_focus_surface(MetaWaylandPointer* p, void* surface) {
    pointer_focus_clears++;
    p->focus_surface = surface;
}
static void meta_wayland_compositor_update(MetaWaylandCompositor* c, const ClutterEvent* e) {
    seat_updates++;
}
static int clutter_input_device_get_mode_switch_button_group(ClutterInputDevice* d, guint b) {
    return -1;
}
static gboolean meta_tablet_action_mapper_handle_event(MetaTabletActionMapper* m,
                                                       const ClutterEvent* e) {
    return FALSE;
}
static guint clutter_input_device_get_capabilities(ClutterInputDevice* d) {
    return 0;
}
static void handle_idletime_for_event(MetaDisplay* d, const ClutterEvent* e) {}
static MetaWindow* get_window_for_event(MetaDisplay* d, const ClutterEvent* e, ClutterActor* a) {
    return &window;
}
static void meta_window_set_user_time(MetaWindow* w, guint t) {}
static void meta_display_sanity_check_timestamps(MetaDisplay* d, guint t) {}
static gboolean meta_keybindings_process_event(MetaDisplay* d, MetaWindow* w,
                                               const ClutterEvent* e) {
    return binding;
}
static gboolean meta_window_handle_ungrabbed_event(MetaWindow* w, const ClutterEvent* e) {
    if (e->type == CLUTTER_BUTTON_PRESS)
        window_presses++;
    return FALSE;
}
static gboolean meta_close_dialog_is_visible(void* d) {
    return FALSE;
}
static guint clutter_event_get_time(const ClutterEvent* e) {
    return 1;
}
static void meta_window_check_alive_on_event(MetaWindow* w, guint t) {}
static gboolean meta_wayland_compositor_handle_event(MetaWaylandCompositor* c,
                                                     const ClutterEvent* e) {
    return meta_wayland_pointer_handle_event(c->pointer, e);
}
static void gnoblin_native_control_observe_key_event(MetaDisplay* d, const ClutterEvent* e) {}
static ClutterContext* _clutter_context_get_default(void) {
    return &context;
}
static ClutterActor* clutter_actor_get_stage(ClutterActor* a) {
    return &context;
}
static ClutterContext* clutter_actor_get_context(ClutterActor* a) {
    return &context;
}
static gboolean clutter_stage_filter_gnoblin_input_policy(ClutterStage* s, const ClutterEvent* e) {
    policy_calls++;
    return policy_queued && !replay;
}
static void clutter_stage_update_device_for_event(ClutterStage* s, ClutterEvent* e) {}
static ClutterActor* clutter_stage_get_event_actor(ClutterStage* s, const ClutterEvent* e) {
    return &window;
}
static ClutterBackend* clutter_context_get_backend(ClutterContext* c) {
    return &context;
}
static ClutterSprite* clutter_backend_get_sprite(ClutterBackend* b, ClutterStage* s,
                                                 ClutterEvent* e) {
    return &context;
}
static void clutter_stage_maybe_lost_implicit_grab(ClutterStage* s, ClutterSprite* p) {}
static void _clutter_stage_process_queued_events(ClutterStage* s) {}
static void handle_motion_event(MetaWaylandPointer* p, const ClutterEvent* e) {
    if (p->focus_surface)
        client_motion++;
}
static void handle_button_event(MetaWaylandPointer* p, const ClutterEvent* e) {
    if (p->focus_surface)
        client_buttons++;
}
static void handle_scroll_event(MetaWaylandPointer* p, const ClutterEvent* e) {}
static void meta_wayland_pointer_gesture_swipe_handle_event(MetaWaylandPointer* p,
                                                            const ClutterEvent* e) {}
static void meta_wayland_pointer_gesture_pinch_handle_event(MetaWaylandPointer* p,
                                                            const ClutterEvent* e) {}
static void meta_wayland_pointer_gesture_hold_handle_event(MetaWaylandPointer* p,
                                                           const ClutterEvent* e) {}

static gboolean diagnostics(const ClutterEvent* e, gpointer data) {
    ui_calls++;
    return panel_handle(e);
}

#if PATCHED
#include "gnoblin-plugin-input.h"
static gboolean modal(gpointer data) {
    return console_modal;
}
static GnoblinPluginInput input = {diagnostics, NULL, NULL, modal};
#endif

static void _clutter_stage_queue_event(ClutterStage* s, ClutterEvent* e, gboolean copy) {
    stage_calls++;
#if !PATCHED
    /* Baseline plugin's stage captured-event handler is reachable only here. */
    if (!locked)
        diagnostics(e, NULL);
#endif
}

#include "routing.inc"

static ClutterEventFilter filter;
static void setup(void) {
    locked = captured = grabbed = replay = consumed = policy_queued = binding = FALSE;
    ui_calls = stage_calls = client_buttons = client_motion = seat_updates = window_presses =
        policy_calls = pointer_focus_clears = 0;
    console_modal = FALSE;
    static GType display_type;
    if (!display_type)
        display_type = g_type_register_static_simple(
            G_TYPE_OBJECT, "TestDisplay", sizeof(GObjectClass), NULL, sizeof(MetaDisplay), NULL, 0);
    display = g_object_new(display_type, NULL);
#if PATCHED
    gnoblin_plugin_input_attach(G_OBJECT(display), &input);
#endif
    pointer.focus_surface = &window;
    filter = (ClutterEventFilter){NULL, event_callback, display};
    context.event_filters = g_list_append(NULL, &filter);
    panel_new();
}
static void cleanup(void) {
    panel_free();
    g_clear_pointer(&context.event_filters, g_list_free);
    g_assert_null(context.current_event);
#if PATCHED
    gnoblin_plugin_input_detach(G_OBJECT(display), &input);
#endif
    g_object_unref(display);
}
static void send_event(ClutterEventType type, float x, float y) {
    ClutterEvent e = {type, x, y, 1};
    clutter_stage_handle_event(&context, &e);
    panel_frame();
}
static void test_dismiss(void) {
    setup();
    float x, y;
    panel_dismiss_coords(&x, &y);
    send_event(CLUTTER_MOTION, x, y);
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    send_event(CLUTTER_BUTTON_RELEASE, x, y);
    g_test_message("UI calls=%u stage calls=%u client buttons=%u", ui_calls, stage_calls,
                   client_buttons);
    g_assert_false(panel_visible());
    g_assert_cmpuint(client_buttons, ==, 0);
    g_assert_cmpuint(window_presses, ==, 0);
    g_assert_cmpuint(seat_updates, ==, 3);
    g_assert_cmpuint(ui_calls, ==, 3);
    cleanup();
}
static void test_outside(void) {
    setup();
    send_event(CLUTTER_MOTION, 700, 500);
    send_event(CLUTTER_BUTTON_PRESS, 700, 500);
    send_event(CLUTTER_BUTTON_RELEASE, 700, 500);
    g_assert_true(panel_visible());
    g_assert_cmpuint(client_buttons, ==, 2);
    g_assert_cmpuint(window_presses, ==, 1);
    g_assert_cmpuint(seat_updates, ==, 3);
    cleanup();
}
static void test_embargoes(void) {
    setup();
    float x, y;
    panel_dismiss_coords(&x, &y);
    locked = TRUE;
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    locked = FALSE;
    captured = TRUE;
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    captured = FALSE;
    replay = consumed = TRUE;
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    consumed = FALSE;
    grabbed = TRUE;
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    replay = grabbed = FALSE;
    policy_queued = TRUE;
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    g_assert_cmpuint(ui_calls, ==, 0);
    g_assert_cmpuint(client_buttons, ==, 0);
    g_assert_true(panel_visible());
    cleanup();
}

static void test_release_outside(void) {
    setup();
    float x, y;
    panel_dismiss_coords(&x, &y);
    send_event(CLUTTER_MOTION, x, y);
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    send_event(CLUTTER_MOTION, 700, 500);
    send_event(CLUTTER_BUTTON_RELEASE, 700, 500);
    g_assert_true(panel_visible());
    g_assert_cmpuint(client_buttons, ==, 0);
    send_event(CLUTTER_BUTTON_PRESS, 700, 500);
    send_event(CLUTTER_BUTTON_RELEASE, 700, 500);
    g_assert_cmpuint(client_buttons, ==, 2);
    cleanup();
}

static void test_unowned_release(void) {
    setup();
    float x, y;
    panel_dismiss_coords(&x, &y);
    send_event(CLUTTER_BUTTON_PRESS, 700, 500);
    send_event(CLUTTER_MOTION, x, y);
    send_event(CLUTTER_BUTTON_RELEASE, x, y);
    g_assert_true(panel_visible());
    g_assert_cmpuint(client_buttons, ==, 2);
    cleanup();
}

static void test_no_client_no_duplicate(void) {
    setup();
    pointer.focus_surface = NULL;
    float x, y;
    panel_dismiss_coords(&x, &y);
    send_event(CLUTTER_MOTION, x, y);
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    send_event(CLUTTER_BUTTON_RELEASE, x, y);
    g_assert_false(panel_visible());
    g_assert_cmpuint(ui_calls, ==, 3);
    g_assert_cmpuint(stage_calls, ==, 1); /* unconsumed hover only */
    cleanup();
}

static void test_dismissed_passes_through(void) {
    setup();
    float x, y;
    panel_dismiss_coords(&x, &y);
    send_event(CLUTTER_MOTION, x, y);
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    send_event(CLUTTER_BUTTON_RELEASE, x, y);
    g_assert_false(panel_visible());
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    send_event(CLUTTER_BUTTON_RELEASE, x, y);
    g_assert_cmpuint(client_buttons, ==, 2);
    cleanup();
}

static void test_replay_allowed(void) {
    setup();
    policy_queued = TRUE;
    float x, y;
    panel_dismiss_coords(&x, &y);
    send_event(CLUTTER_MOTION, x, y);
    g_assert_cmpuint(ui_calls, ==, 0);
    replay = TRUE;
    send_event(CLUTTER_MOTION, x, y);
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    send_event(CLUTTER_BUTTON_RELEASE, x, y);
    g_assert_false(panel_visible());
    g_assert_cmpuint(ui_calls, ==, 3);
    g_assert_cmpuint(client_buttons, ==, 0);
    cleanup();
}

/* While the console is open the pointer belongs to it: no client gets hover, enter, motion or
 * buttons, the seat is not updated, and the client that had pointer focus loses it. */
static void test_console_modal(void) {
    setup();
    console_modal = TRUE;
    send_event(CLUTTER_MOTION, 700, 500);
    send_event(CLUTTER_BUTTON_PRESS, 700, 500);
    send_event(CLUTTER_BUTTON_RELEASE, 700, 500);
    send_event(CLUTTER_SCROLL, 700, 500);
    g_assert_cmpuint(ui_calls, ==, 4);
    g_assert_cmpuint(seat_updates, ==, 0);
    g_assert_cmpuint(client_buttons, ==, 0);
    g_assert_cmpuint(client_motion, ==, 0);
    g_assert_cmpuint(window_presses, ==, 0);
    g_assert_null(pointer.focus_surface);
    g_assert_cmpuint(pointer_focus_clears, ==, 4);
    /* Closing the console hands the pointer straight back to clients. */
    console_modal = FALSE;
    pointer.focus_surface = &window;
    send_event(CLUTTER_BUTTON_PRESS, 700, 500);
    send_event(CLUTTER_BUTTON_RELEASE, 700, 500);
    g_assert_cmpuint(client_buttons, ==, 2);
    g_assert_cmpuint(pointer_focus_clears, ==, 4);
    cleanup();
}

static void test_detached(void) {
    setup();
#if PATCHED
    gnoblin_plugin_input_detach(G_OBJECT(display), &input);
#endif
    float x, y;
    panel_dismiss_coords(&x, &y);
    send_event(CLUTTER_BUTTON_PRESS, x, y);
    send_event(CLUTTER_BUTTON_RELEASE, x, y);
    g_assert_true(panel_visible());
    g_assert_cmpuint(ui_calls, ==, 0);
    g_assert_cmpuint(client_buttons, ==, 2);
    cleanup();
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/recovery/dismiss-through-native-route", test_dismiss);
    g_test_add_func("/recovery/outside-passes-through", test_outside);
    g_test_add_func("/recovery/security-embargoes", test_embargoes);
    g_test_add_func("/recovery/release-outside", test_release_outside);
    g_test_add_func("/recovery/unowned-release", test_unowned_release);
    g_test_add_func("/recovery/no-client-no-duplicate", test_no_client_no_duplicate);
    g_test_add_func("/recovery/dismissed-passes-through", test_dismissed_passes_through);
    g_test_add_func("/recovery/replay-allowed", test_replay_allowed);
    g_test_add_func("/recovery/console-modal-pointer", test_console_modal);
    g_test_add_func("/recovery/detached", test_detached);
    return g_test_run();
}
