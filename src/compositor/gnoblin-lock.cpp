/*
 * gnoblin-shell: a session lock screen (overlay + ClutterGrab + PAM).
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-lock.h"

#include <gio/gio.h>
#include <security/pam_appl.h>
#include <string.h>

extern "C" {
#include <clutter/clutter-pango.h>
#include <clutter/clutter.h>
#include <meta/compositor.h>
#include <meta/display.h>
#include <meta/meta-backend.h>
#include <meta/meta-context.h>
#include <meta/meta-monitor-manager.h>
}

#include "gnoblin-config.h"
#include "gnoblin-control.h"

/* One lock at a time, kept process-wide so the keybind/D-Bus can't stack them. */
typedef struct {
    ClutterActor* overlay;
    ClutterActor* prompt_region;
    ClutterActor* prompt;
    ClutterActor* entry; /* Display-only masked password field */
    ClutterActor* hint;  /* ClutterText status line */
    ClutterGrab* grab;
    MetaDisplay* display;
    MetaMonitorManager* monitor_manager;
    gulong monitors_changed_id;
    GString* password;
    gboolean auth_in_progress;
    guint64 lock_id;
} GnoblinLock;

static GnoblinLock* the_lock;
static guint64 next_lock_id;
static void on_overlay_destroyed(ClutterActor* overlay, gpointer user_data);
static void lock_dismiss(void);
static void update_password_text(void);

typedef struct {
    char* password;
    guint64 lock_id;
} AuthRequest;

static void auth_request_free(AuthRequest* request) {
    if (!request)
        return;
    g_free(request->password);
    g_free(request);
}

static void update_prompt_region(GnoblinLock* lock) {
    MtkRectangle rect = {0, 0, 0, 0};

    if (!lock || !lock->display || !lock->prompt_region)
        return;

    if (meta_display_get_n_monitors(lock->display) > 0) {
        int monitor = meta_display_get_primary_monitor(lock->display);

        meta_display_get_monitor_geometry(lock->display, monitor, &rect);
    } else if (lock->overlay) {
        float width = 0, height = 0;

        clutter_actor_get_size(lock->overlay, &width, &height);
        rect.width = (int)width;
        rect.height = (int)height;
    }

    clutter_actor_set_position(lock->prompt_region, rect.x, rect.y);
    clutter_actor_set_size(lock->prompt_region, rect.width, rect.height);
}

static void on_monitors_changed(MetaMonitorManager* monitor_manager, gpointer user_data) {
    update_prompt_region((GnoblinLock*)user_data);
}

/* ---- PAM ---- */

static int lock_conv(int num_msg, const struct pam_message** msg, struct pam_response** resp,
                     void* appdata) {
    const char* password = (const char*)appdata;
    struct pam_response* replies;
    int i;

    if (num_msg <= 0)
        return PAM_CONV_ERR;

    replies = (struct pam_response*)calloc(num_msg, sizeof(struct pam_response));
    if (!replies)
        return PAM_BUF_ERR;

    for (i = 0; i < num_msg; i++) {
        /* Answer every prompt with the typed password; ignore info/error msgs. */
        if (msg[i]->msg_style == PAM_PROMPT_ECHO_OFF || msg[i]->msg_style == PAM_PROMPT_ECHO_ON)
            replies[i].resp = strdup(password ? password : "");
    }

    *resp = replies;
    return PAM_SUCCESS;
}

/* Verify the current user's password. Blocks briefly (PAM runs unix_chkpwd). */
static gboolean lock_authenticate(const char* password) {
    g_autofree char* service = gnoblin_config_get_string("lock", "pam-service");
    const char* user = g_get_user_name();
    struct pam_conv conv = {lock_conv, (void*)password};
    pam_handle_t* handle = NULL;
    int rc;

    if (!service)
        service = g_strdup("login"); /* a service present on essentially every distro */

    if (pam_start(service, user, &conv, &handle) != PAM_SUCCESS)
        return FALSE;

    rc = pam_authenticate(handle, 0);
    pam_end(handle, rc);

    return rc == PAM_SUCCESS;
}

static void authenticate_task(GTask* task, gpointer source_object, gpointer task_data,
                              GCancellable* cancellable) {
    AuthRequest* request = (AuthRequest*)task_data;

    g_task_return_boolean(task, lock_authenticate(request->password));
}

static void authenticate_done(GObject* source_object, GAsyncResult* result, gpointer user_data) {
    GTask* task = G_TASK(result);
    AuthRequest* request = (AuthRequest*)g_task_get_task_data(task);
    g_autoptr(GError) error = NULL;
    gboolean authenticated;

    authenticated = g_task_propagate_boolean(task, &error);

    if (!the_lock || the_lock->lock_id != request->lock_id)
        return;

    the_lock->auth_in_progress = FALSE;

    if (authenticated) {
        lock_dismiss();
        return;
    }

    /* Wrong password: clear the field and nudge the user. */
    g_string_truncate(the_lock->password, 0);
    update_password_text();
    if (the_lock && the_lock->hint)
        clutter_text_set_text(CLUTTER_TEXT(the_lock->hint), "Incorrect password");
}

/* ---- overlay ---- */

static void free_lock(GnoblinLock* lock, gboolean destroy_overlay) {
    if (!lock)
        return;

    if (lock->grab) {
        clutter_grab_dismiss(lock->grab);
        g_clear_object(&lock->grab);
    }
    if (lock->monitor_manager && lock->monitors_changed_id) {
        g_signal_handler_disconnect(lock->monitor_manager, lock->monitors_changed_id);
        lock->monitors_changed_id = 0;
    }
    if (destroy_overlay && lock->overlay) {
        g_signal_handlers_disconnect_by_func(lock->overlay, (gpointer)on_overlay_destroyed, NULL);
        clutter_actor_destroy(lock->overlay);
    }
    if (lock->password)
        g_string_free(lock->password, TRUE);
    g_free(lock);
}

static void on_overlay_destroyed(ClutterActor* overlay, gpointer user_data) {
    GnoblinLock* lock = the_lock;

    if (!lock || lock->overlay != overlay)
        return;

    lock->overlay = NULL;
    lock->prompt_region = NULL;
    lock->prompt = NULL;
    lock->entry = NULL;
    lock->hint = NULL;
    the_lock = NULL;
    free_lock(lock, FALSE);
    gnoblin_control_set_locked(FALSE);
}

static void lock_dismiss(void) {
    GnoblinLock* lock = the_lock;

    if (!lock)
        return;

    the_lock = NULL;
    if (lock->overlay)
        g_signal_handlers_disconnect_by_func(lock->overlay, (gpointer)on_overlay_destroyed, NULL);
    free_lock(lock, TRUE);
    gnoblin_control_set_locked(FALSE);
}

static void update_password_text(void) {
    gsize len;
    g_autofree char* masked = NULL;

    if (!the_lock || !the_lock->entry || !the_lock->password)
        return;

    len = g_utf8_strlen(the_lock->password->str, -1);
    masked = g_strnfill(len, '*');
    clutter_text_set_text(CLUTTER_TEXT(the_lock->entry), masked ? masked : "");
}

static void submit_password(void) {
    AuthRequest* request;
    GTask* task;

    if (!the_lock || !the_lock->password)
        return;
    if (the_lock->auth_in_progress)
        return;

    request = g_new0(AuthRequest, 1);
    request->password = g_strdup(the_lock->password->str);
    request->lock_id = the_lock->lock_id;
    the_lock->auth_in_progress = TRUE;
    if (the_lock && the_lock->hint)
        clutter_text_set_text(CLUTTER_TEXT(the_lock->hint), "Checking password...");

    task = g_task_new(NULL, NULL, authenticate_done, NULL);
    g_task_set_task_data(task, request, (GDestroyNotify)auth_request_free);
    g_task_run_in_thread(task, authenticate_task);
    g_object_unref(task);
}

static void append_password_char(gunichar ch) {
    char buf[7] = {0};
    int n;

    if (!the_lock || !the_lock->password || !g_unichar_isprint(ch))
        return;

    n = g_unichar_to_utf8(ch, buf);
    buf[n] = '\0';
    g_string_append(the_lock->password, buf);
    update_password_text();
}

static void delete_password_char(void) {
    char* prev;

    if (!the_lock || !the_lock->password || the_lock->password->len == 0)
        return;

    prev = g_utf8_find_prev_char(the_lock->password->str,
                                 the_lock->password->str + the_lock->password->len);
    if (prev)
        g_string_truncate(the_lock->password, prev - the_lock->password->str);
    else
        g_string_truncate(the_lock->password, 0);
    update_password_text();
}

static gboolean on_lock_event(ClutterActor* overlay, ClutterEvent* event, gpointer user_data) {
    guint sym;

    if (clutter_event_type(event) == CLUTTER_ENTER || clutter_event_type(event) == CLUTTER_LEAVE)
        return CLUTTER_EVENT_PROPAGATE;
    if (clutter_event_type(event) != CLUTTER_KEY_PRESS)
        return CLUTTER_EVENT_STOP;
    if (the_lock && the_lock->auth_in_progress)
        return CLUTTER_EVENT_STOP;

    sym = clutter_event_get_key_symbol(event);
    switch (sym) {
    case CLUTTER_KEY_Return:
    case CLUTTER_KEY_KP_Enter:
        submit_password();
        return CLUTTER_EVENT_STOP;
    case CLUTTER_KEY_BackSpace:
        delete_password_char();
        return CLUTTER_EVENT_STOP;
    case CLUTTER_KEY_Escape:
        return CLUTTER_EVENT_STOP;
    default:
        append_password_char(clutter_event_get_key_unicode(event));
        return CLUTTER_EVENT_STOP;
    }
}

void gnoblin_lock_engage(MetaDisplay* display) {
    MetaCompositor* compositor = meta_display_get_compositor(display);
    ClutterActor* stage = CLUTTER_ACTOR(meta_compositor_get_stage(compositor));
    MetaContext* context = meta_display_get_context(display);
    MetaBackend* backend = meta_context_get_backend(context);
    CoglColor backdrop = (CoglColor)COGL_COLOR_INIT(16, 17, 19, 255);
    CoglColor field = (CoglColor)COGL_COLOR_INIT(40, 42, 46, 255);
    CoglColor text = (CoglColor)COGL_COLOR_INIT(255, 255, 255, 255);
    CoglColor dim = (CoglColor)COGL_COLOR_INIT(200, 200, 200, 200);
    float width = 0, height = 0;

    if (the_lock)
        return;

    clutter_actor_get_size(stage, &width, &height);

    the_lock = g_new0(GnoblinLock, 1);
    the_lock->lock_id = ++next_lock_id;
    the_lock->display = display;
    the_lock->monitor_manager = meta_backend_get_monitor_manager(backend);
    the_lock->password = g_string_new(NULL);

    /* Opaque backdrop across the whole stage, on top of every window. */
    the_lock->overlay = clutter_actor_new();
    clutter_actor_set_background_color(the_lock->overlay, &backdrop);
    clutter_actor_set_size(the_lock->overlay, width, height);
    clutter_actor_add_constraint(the_lock->overlay,
                                 clutter_bind_constraint_new(stage, CLUTTER_BIND_WIDTH, 0));
    clutter_actor_add_constraint(the_lock->overlay,
                                 clutter_bind_constraint_new(stage, CLUTTER_BIND_HEIGHT, 0));
    clutter_actor_set_reactive(the_lock->overlay, TRUE);
    clutter_actor_add_child(stage, the_lock->overlay);
    g_signal_connect(the_lock->overlay, "destroy", G_CALLBACK(on_overlay_destroyed), NULL);
    g_signal_connect(the_lock->overlay, "event", G_CALLBACK(on_lock_event), NULL);

    /* A clock-ish prompt + a masked password entry, centred on the primary monitor. */
    the_lock->prompt_region = clutter_actor_new();
    clutter_actor_set_reactive(the_lock->prompt_region, FALSE);
    update_prompt_region(the_lock);
    clutter_actor_add_child(the_lock->overlay, the_lock->prompt_region);
    the_lock->monitors_changed_id =
        g_signal_connect(the_lock->monitor_manager, "monitors-changed",
                         G_CALLBACK(on_monitors_changed), the_lock);

    the_lock->prompt = clutter_actor_new();
    clutter_actor_set_size(the_lock->prompt, 280, 84);
    clutter_actor_set_reactive(the_lock->prompt, FALSE);
    clutter_actor_add_constraint(the_lock->prompt,
                                 clutter_align_constraint_new(the_lock->prompt_region,
                                                              CLUTTER_ALIGN_BOTH, 0.5));
    clutter_actor_add_child(the_lock->prompt_region, the_lock->prompt);

    the_lock->hint = clutter_text_new_full("Sans 12", "Locked — enter password", &dim);
    clutter_actor_set_reactive(the_lock->hint, FALSE);
    clutter_actor_set_position(the_lock->hint, 20, 0);
    clutter_actor_add_child(the_lock->prompt, the_lock->hint);

    the_lock->entry = clutter_text_new_full("Sans 14", "", &text);
    clutter_actor_set_background_color(the_lock->entry, &field);
    clutter_actor_set_size(the_lock->entry, 280, 36);
    clutter_actor_set_reactive(the_lock->entry, FALSE);
    clutter_actor_set_position(the_lock->entry, 0, 48);
    clutter_actor_add_child(the_lock->prompt, the_lock->entry);

    /* Route ALL seat input to the overlay: windows can't be reached while held. */
    the_lock->grab = clutter_stage_grab(CLUTTER_STAGE(stage), the_lock->overlay);
    clutter_actor_grab_key_focus(the_lock->overlay);

    /* Suppress config keybindings so e.g. Super+Space can't bypass the lock. */
    gnoblin_control_set_locked(TRUE);
}
