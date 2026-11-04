/* The leased keyboard grab of the Lua runtime.
 *
 * gnoblin.input.grab() takes every key event: while a grab exists no application, no shortcut and
 * no other Mutter handler sees a key press or release. The grab is a lease. It ends by itself when
 * its idle timeout runs out, so a script that stops or hangs cannot keep the keyboard for ever.
 * Each key event and each input.grab.renew restarts the timeout; the user typing shows that
 * someone still needs the grab.
 *
 *   input.grab           {timeout}  take the grab: one slot for the whole compositor
 *   input.grab.renew     {id}       restart the idle timeout
 *   input.grab.release   {id}       give the keyboard back
 *
 * The compositor tells the runtime that owns the grab:
 *
 *   gnoblin.input.grab.key     {id, key, keysym, keycode, modifiers, pressed, repeated,
 *                               composing, time, text?}
 *   gnoblin.input.grab.closed  {id, reason}
 *
 * The reasons are released, expired, replaced (another grab took the slot), locked (the session
 * locked), preempted (another owner took the keyboard, such as a popup grab), config_reloaded and
 * runtime_stopped. A reload that commits, a stopped worker and a stopped compositor end the grab
 * without an event: the Lua runtime that held it is gone or ends its own handle.
 *
 * Boundaries. A grab is the Lua runtime's, not a user gesture, so its events carry no focus
 * context: a script cannot use the keys it holds to move or focus a window. The grab refuses to
 * start while the session is locked and ends when the session locks. It shares the one native key
 * capture slot of Mutter with the shortcut sessions (modal and capture_input shortcuts) and
 * shortcuts.capture, and never takes it from them: a grab refuses to start while one holds it,
 * and while a grab holds it they cannot start.
 *
 * Text. A press carries `text` when it types something. The compositor feeds each press to the
 * compose table of the locale (xkbcommon), so a dead key or a Compose sequence yields one
 * composed character on the key that completes it, and `composing` is true on the keys inside the
 * sequence, and the key that breaks a sequence has its own text. A key with no text of its own,
 * or one pressed with Control, Alt or Super, has none. */
#include "config.h"

#include <xkbcommon/xkbcommon-compose.h>
#include <xkbcommon/xkbcommon.h>

#include "core/gnoblin-control-internal.h"
#include "core/gnoblin-native-control-private.h"

#define GRAB_DEFAULT_TIMEOUT 10
#define GRAB_MAX_TIMEOUT 60

/* Modifiers that make a key a command, not typing. */
#define GRAB_COMMAND_MASK (CLUTTER_CONTROL_MASK | CLUTTER_MOD1_MASK | CLUTTER_SUPER_MASK)

typedef struct {
    GnoblinNativeControl* control;
    char* id;
    guint64 generation; /* the runtime generation that owns it */
    guint timeout;      /* idle seconds */
    guint timer_id;
    struct xkb_compose_state* compose; /* NULL when the locale has no compose table */
} InputGrab;

static void grab_free(InputGrab* grab) {
    if (grab->timer_id)
        g_source_remove(grab->timer_id);
    g_clear_pointer(&grab->compose, xkb_compose_state_unref);
    g_free(grab->id);
    g_free(grab);
}

static void grab_publish_closed(GnoblinNativeControl* control, const InputGrab* grab,
                                const char* reason) {
    if (grab->generation != control->runtime_generation || control->stopping)
        return;
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "id", g_variant_new_string(grab->id));
    g_variant_builder_add(&payload, "{sv}", "reason", g_variant_new_string(reason));
    g_variant_builder_add(&payload, "{sv}", "sequence",
                          g_variant_new_int64(++control->event_sequence));
    g_variant_builder_add(&payload, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    g_autoptr(GVariant) event = g_variant_ref_sink(g_variant_builder_end(&payload));
    native_runtime_dispatch_event(control, "gnoblin.input.grab.closed", event);
}

/* End the grab: hand the keyboard back, then (with `publish`) tell the runtime why. */
void gnoblin_control_input_grab_end(GnoblinNativeControl* control, const char* reason,
                                    gboolean publish) {
    InputGrab* grab = control->input_grab;
    if (!grab)
        return;
    control->input_grab = NULL;
    if (control->display) {
        meta_display_stop_native_key_capture(control->display);
        meta_display_unregister_native_key_capture_handler(control->display, control);
    }
    if (publish)
        grab_publish_closed(control, grab, reason);
    grab_free(grab);
}

static gboolean grab_expired(gpointer user_data) {
    InputGrab* grab = user_data;
    grab->timer_id = 0;
    gnoblin_control_input_grab_end(grab->control, "expired", TRUE);
    return G_SOURCE_REMOVE;
}

static void grab_arm(InputGrab* grab) {
    if (grab->timer_id)
        g_source_remove(grab->timer_id);
    grab->timer_id = g_timeout_add(grab->timeout * 1000, grab_expired, grab);
}

/* The compose table of the locale, loaded once. The locale is the one the environment names, as
 * libxkbcommon documents; "C" uses the en_US.UTF-8 table. NULL when there is none. */
static struct xkb_compose_table* compose_table(GnoblinNativeControl* control) {
    if (!control->compose_table) {
        const char* locale = g_getenv("LC_ALL");
        if (!locale || !*locale)
            locale = g_getenv("LC_CTYPE");
        if (!locale || !*locale)
            locale = g_getenv("LANG");
        if (!locale || !*locale)
            locale = "C";
        struct xkb_context* context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
        if (context) {
            control->compose_table =
                xkb_compose_table_new_from_locale(context, locale, XKB_COMPOSE_COMPILE_NO_FLAGS);
            xkb_context_unref(context); /* the table keeps its own reference */
        }
        if (!control->compose_table)
            g_warning("gnoblin-native-control: no compose table for locale %s; key grabs report "
                      "no composed text",
                      locale);
    }
    return control->compose_table;
}

/* The text of a printable keysym, or NULL for a key that types nothing (a modifier, an arrow, a
 * control character such as Return or Tab). */
static char* keysym_text(xkb_keysym_t keysym) {
    char buffer[8];
    int length = xkb_keysym_to_utf8(keysym, buffer, sizeof buffer);
    if (length <= 1)
        return NULL;
    gunichar character = g_utf8_get_char_validated(buffer, -1);
    if (character == (gunichar)-1 || character == (gunichar)-2 || g_unichar_iscntrl(character))
        return NULL;
    return g_strdup(buffer);
}

/* What a press types. A compose sequence in progress types nothing and sets `composing`; the key
 * that completes it types the composed text; a key that cancels it types its own text. */
static char* press_text(InputGrab* grab, xkb_keysym_t keysym, guint32 state, gboolean* composing) {
    *composing = FALSE;
    if (state & GRAB_COMMAND_MASK) {
        if (grab->compose)
            xkb_compose_state_reset(grab->compose);
        return NULL;
    }
    if (grab->compose) {
        xkb_compose_state_feed(grab->compose, keysym);
        switch (xkb_compose_state_get_status(grab->compose)) {
        case XKB_COMPOSE_COMPOSING:
            *composing = TRUE;
            return NULL;
        case XKB_COMPOSE_COMPOSED: {
            char buffer[64];
            int length = xkb_compose_state_get_utf8(grab->compose, buffer, sizeof buffer);
            xkb_compose_state_reset(grab->compose);
            return length > 0 && length < (int)sizeof buffer ? g_strdup(buffer) : NULL;
        }
        case XKB_COMPOSE_CANCELLED:
            /* The sequence is dropped; the key that broke it still types what it types. */
            xkb_compose_state_reset(grab->compose);
            break;
        case XKB_COMPOSE_NOTHING:
            break;
        }
    }
    return keysym_text(keysym);
}

/* Keep the set of keys that are physically down. The stage input filter sees every key event
 * first, also while a grab or a shortcut capture owns the keyboard, so the set stays true. A grab
 * reads it: Mutter sends the release of a held key to the grab, and the application that saw the
 * press would never see the key lift. */
void gnoblin_control_track_pressed_key(GnoblinNativeControl* control, const ClutterEvent* event) {
    if (!control->pressed_keys || !event)
        return;
    ClutterEventType type = clutter_event_type(event);
    if (type != CLUTTER_KEY_PRESS && type != CLUTTER_KEY_RELEASE)
        return;
    if (clutter_event_get_flags(event) &
        (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD))
        return;
    gpointer code = GUINT_TO_POINTER(clutter_event_get_key_code(event));
    if (type == CLUTTER_KEY_RELEASE) {
        g_hash_table_remove(control->pressed_keys, code);
        return;
    }
    if (g_hash_table_contains(control->pressed_keys, code))
        return;
    xkb_keysym_t keysym = clutter_event_get_key_symbol(event);
    char name[64] = {0};
    if (xkb_keysym_get_name(keysym, name, sizeof name) < 0)
        g_snprintf(name, sizeof name, "0x%x", keysym);
    g_hash_table_insert(control->pressed_keys, code, g_strdup(name));
}

/* The native key capture handler's grab branch: `event` is a key event, or NULL when Mutter
 * takes the keyboard back (a popup or other stage grab opened). */
void gnoblin_control_input_grab_key(GnoblinNativeControl* control, const ClutterEvent* event) {
    InputGrab* grab = control->input_grab;
    if (!grab)
        return;
    if (!event) {
        gnoblin_control_input_grab_end(control, "preempted", TRUE);
        return;
    }
    ClutterEventType type = clutter_event_type(event);
    if (type != CLUTTER_KEY_PRESS && type != CLUTTER_KEY_RELEASE)
        return;
    ClutterEventFlags flags = clutter_event_get_flags(event);
    if (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD))
        return;
    gboolean pressed = type == CLUTTER_KEY_PRESS;
    xkb_keysym_t keysym = clutter_event_get_key_symbol(event);
    guint32 state = clutter_event_get_state(event);
    gboolean composing = FALSE;
    g_autofree char* text = pressed ? press_text(grab, keysym, state, &composing) : NULL;
    char name[64] = {0};
    if (xkb_keysym_get_name(keysym, name, sizeof name) < 0)
        g_snprintf(name, sizeof name, "0x%x", keysym);

    grab_arm(grab); /* a key is a sign that the grab is still wanted */
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(grab->id));
    g_variant_builder_add(&builder, "{sv}", "key", g_variant_new_string(name));
    g_variant_builder_add(&builder, "{sv}", "keysym", g_variant_new_uint32(keysym));
    g_variant_builder_add(&builder, "{sv}", "keycode",
                          g_variant_new_uint32(clutter_event_get_key_code(event)));
    g_variant_builder_add(&builder, "{sv}", "modifiers", g_variant_new_uint32(state));
    g_variant_builder_add(&builder, "{sv}", "pressed", g_variant_new_boolean(pressed));
    g_variant_builder_add(&builder, "{sv}", "repeated",
                          g_variant_new_boolean((flags & CLUTTER_EVENT_FLAG_REPEATED) != 0));
    g_variant_builder_add(&builder, "{sv}", "composing", g_variant_new_boolean(composing));
    if (text)
        g_variant_builder_add(&builder, "{sv}", "text", g_variant_new_string(text));
    g_variant_builder_add(&builder, "{sv}", "time",
                          g_variant_new_int64(clutter_event_get_time(event)));
    g_variant_builder_add(&builder, "{sv}", "sequence",
                          g_variant_new_int64(++control->event_sequence));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    native_runtime_dispatch_event(control, "gnoblin.input.grab.key", payload);
}

/* The grab `arguments` name, if this generation owns it; otherwise NULL and an error. */
static InputGrab* find_grab(GnoblinNativeControl* control, const char* method, GVariant* arguments,
                            guint64 owner_generation, GError** error) {
    const char* id = NULL;
    if (!g_variant_lookup(arguments, "id", "&s", &id) || !*id) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "%s needs an id", method);
        return NULL;
    }
    InputGrab* grab = control->input_grab;
    if (!grab || grab->generation != owner_generation || !g_str_equal(grab->id, id)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "grab %s has ended", id);
        return NULL;
    }
    return grab;
}

static GVariant* grab_result(const char* field, gboolean value, const InputGrab* grab) {
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(grab->id));
    g_variant_builder_add(&result, "{sv}", field, g_variant_new_boolean(value));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

/* input.grab {timeout}: take the keyboard for the runtime generation that asked. */
GVariant* gnoblin_control_input_grab(GnoblinNativeControl* control, gint64 operation_id,
                                     GVariant* arguments, guint64 owner_generation,
                                     GError** error) {
    if (owner_generation != control->runtime_generation) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "input.grab belongs to an old runtime generation");
        return NULL;
    }
    gint64 timeout = GRAB_DEFAULT_TIMEOUT;
    g_autoptr(GVariant) given = g_variant_lookup_value(arguments, "timeout", NULL);
    if (given && !g_variant_is_of_type(given, G_VARIANT_TYPE_INT64) &&
        !g_variant_is_of_type(given, G_VARIANT_TYPE_DOUBLE)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.grab timeout must be a number of seconds");
        return NULL;
    }
    if (given)
        timeout = g_variant_is_of_type(given, G_VARIANT_TYPE_INT64)
                      ? g_variant_get_int64(given)
                      : (gint64)g_variant_get_double(given);
    if (timeout < 1 || timeout > GRAB_MAX_TIMEOUT) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "input.grab timeout must be from 1 to %d seconds", GRAB_MAX_TIMEOUT);
        return NULL;
    }
    if (!control->display || control->stopping) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "keyboard grabs are unavailable");
        return NULL;
    }
    /* The lock screen owns the keyboard. A grab would also see nothing there: Mutter does not
     * call the capture handler while the session is locked. */
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "keyboard input cannot be grabbed while the session is locked");
        return NULL;
    }
    if (control->active_shortcut_session || control->shortcut_capture_active) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "a shortcut session or shortcut capture is holding the keyboard");
        return NULL;
    }
    /* Mutter gives the release of a key held now to the grab, so the application that saw the
     * press would never see the key lift. Refuse until every key is up. */
    if (control->pressed_keys && g_hash_table_size(control->pressed_keys) > 0) {
        GHashTableIter held;
        gpointer name = NULL;
        g_hash_table_iter_init(&held, control->pressed_keys);
        g_hash_table_iter_next(&held, NULL, &name);
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                    "keyboard input cannot be grabbed while a key is held (%s); its release would "
                    "go to the grab and the focused application would never see the key lift. "
                    "Start the grab when every key is up",
                    (const char*)name);
        return NULL;
    }
    gnoblin_control_input_grab_end(control, "replaced", TRUE);
    if (!meta_display_can_start_native_key_capture(control->display) ||
        !meta_display_register_native_key_capture_handler(
            control->display, native_shortcut_capture_key, control, NULL)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "keyboard input is already grabbed or captured");
        return NULL;
    }
    if (!meta_display_start_native_key_capture(control->display)) {
        meta_display_unregister_native_key_capture_handler(control->display, control);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another input capture owner is active");
        return NULL;
    }
    InputGrab* grab = g_new0(InputGrab, 1);
    grab->control = control;
    grab->id = g_strdup_printf("grab-%" G_GINT64_FORMAT, operation_id);
    grab->generation = owner_generation;
    grab->timeout = (guint)timeout;
    struct xkb_compose_table* table = compose_table(control);
    if (table)
        grab->compose = xkb_compose_state_new(table, XKB_COMPOSE_STATE_NO_FLAGS);
    control->input_grab = grab;
    grab_arm(grab);

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(grab->id));
    g_variant_builder_add(&result, "{sv}", "timeout", g_variant_new_int64(timeout));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

/* input.grab.renew {id}: restart the idle timeout of the grab. */
GVariant* gnoblin_control_input_grab_renew(GnoblinNativeControl* control, GVariant* arguments,
                                           guint64 owner_generation, GError** error) {
    InputGrab* grab = find_grab(control, "input.grab.renew", arguments, owner_generation, error);
    if (!grab)
        return NULL;
    grab_arm(grab);
    return grab_result("renewed", TRUE, grab);
}

/* input.grab.release {id}: end the grab and give the keyboard back. */
GVariant* gnoblin_control_input_grab_release(GnoblinNativeControl* control, GVariant* arguments,
                                             guint64 owner_generation, GError** error) {
    InputGrab* grab = find_grab(control, "input.grab.release", arguments, owner_generation, error);
    if (!grab)
        return NULL;
    g_autoptr(GVariant) result = grab_result("released", TRUE, grab);
    gnoblin_control_input_grab_end(control, "released", TRUE);
    return g_steal_pointer(&result);
}

void gnoblin_control_input_grab_free(GnoblinNativeControl* control) {
    gnoblin_control_input_grab_end(control, "runtime_stopped", FALSE);
    g_clear_pointer((struct xkb_compose_table**)&control->compose_table, xkb_compose_table_unref);
}
