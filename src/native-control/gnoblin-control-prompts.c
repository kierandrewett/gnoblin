/* Keyring and GPG passphrase prompter: serves org.gnome.keyring.SystemPrompter and
 * org.gnome.keyring.PrivatePrompter through libgcr and relays each prompt to Lua handlers
 * (prompt.respond, prompt.cancel). It stays off until the prompts.enabled setting is true. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#define GCR_API_SUBJECT_TO_CHANGE
#include <gcr/gcr.h>

#define MAX_PENDING_PROMPTS 8
#define PROMPT_ANSWER_TIMEOUT_SECONDS 120
#define PROMPT_MAX_PASSWORD_BYTES 4096

#define SYSTEM_PROMPTER_NAME "org.gnome.keyring.SystemPrompter"
#define PRIVATE_PROMPTER_NAME "org.gnome.keyring.PrivatePrompter"

typedef enum {
    PROMPT_KIND_PASSWORD,
    PROMPT_KIND_CONFIRM,
} PromptKind;

/* The prompter creates prompt objects itself, so a prompt finds the control through this
 * pointer. It is set only while the broker runs, and there is one control per compositor. */
static GnoblinNativeControl* active_control;

struct _GnoblinControlPromptBroker {
    int ref_count;
    gboolean stopped;
    GnoblinNativeControl* control;
    GCancellable* cancellable;
    GDBusConnection* connection;
    GcrSystemPrompter* prompter;
    guint system_name_id;
    guint private_name_id;
    guint owned_names;
    gboolean name_failed;
};

typedef struct {
    GnoblinNativeControl* control;
    GTask* task;
    GObject* prompt;
    GCancellable* cancellable;
    gulong cancelled_handler_id;
    GHashTable* recipient_client_ids;
    guint64 request_id;
    guint timeout_source_id;
    PromptKind kind;
    const char* result;
} PendingPrompt;

/* GcrPrompt implementation. The prompter sets the properties, then asks for a password or
 * a confirmation. The answer arrives later from prompt.respond or prompt.cancel. */

typedef struct {
    GObject parent;
    char* title;
    char* message;
    char* description;
    char* warning;
    char* choice_label;
    char* caller_window;
    char* continue_label;
    char* cancel_label;
    gboolean choice_chosen;
    gboolean password_new;
    char* password;
} GnoblinControlPrompt;

typedef struct {
    GObjectClass parent_class;
} GnoblinControlPromptClass;

static GType gnoblin_control_prompt_get_type(void);
static void gnoblin_control_prompt_iface_init(GcrPromptInterface* iface);

G_DEFINE_TYPE_WITH_CODE(GnoblinControlPrompt, gnoblin_control_prompt, G_TYPE_OBJECT,
                        G_IMPLEMENT_INTERFACE(GCR_TYPE_PROMPT, gnoblin_control_prompt_iface_init))

#define GNOBLIN_CONTROL_PROMPT(obj) \
    (G_TYPE_CHECK_INSTANCE_CAST((obj), gnoblin_control_prompt_get_type(), GnoblinControlPrompt))

enum {
    PROP_0,
    PROP_TITLE,
    PROP_MESSAGE,
    PROP_DESCRIPTION,
    PROP_WARNING,
    PROP_CHOICE_LABEL,
    PROP_CHOICE_CHOSEN,
    PROP_PASSWORD_NEW,
    PROP_PASSWORD_STRENGTH,
    PROP_CALLER_WINDOW,
    PROP_CONTINUE_LABEL,
    PROP_CANCEL_LABEL,
};

static void prompt_wipe_password(GnoblinControlPrompt* self) {
    if (self->password)
        gcr_secure_memory_strfree(self->password);
    self->password = NULL;
}

static void prompt_set_string(char** field, const GValue* value) {
    g_free(*field);
    *field = g_value_dup_string(value);
}

static void gnoblin_control_prompt_set_property(GObject* object, guint property_id,
                                                const GValue* value, GParamSpec* pspec) {
    GnoblinControlPrompt* self = GNOBLIN_CONTROL_PROMPT(object);
    switch (property_id) {
    case PROP_TITLE:
        prompt_set_string(&self->title, value);
        break;
    case PROP_MESSAGE:
        prompt_set_string(&self->message, value);
        break;
    case PROP_DESCRIPTION:
        prompt_set_string(&self->description, value);
        break;
    case PROP_WARNING:
        prompt_set_string(&self->warning, value);
        break;
    case PROP_CHOICE_LABEL:
        prompt_set_string(&self->choice_label, value);
        break;
    case PROP_CHOICE_CHOSEN:
        self->choice_chosen = g_value_get_boolean(value);
        break;
    case PROP_PASSWORD_NEW:
        self->password_new = g_value_get_boolean(value);
        break;
    case PROP_CALLER_WINDOW:
        prompt_set_string(&self->caller_window, value);
        break;
    case PROP_CONTINUE_LABEL:
        prompt_set_string(&self->continue_label, value);
        break;
    case PROP_CANCEL_LABEL:
        prompt_set_string(&self->cancel_label, value);
        break;
    default:
        G_OBJECT_WARN_INVALID_PROPERTY_ID(object, property_id, pspec);
    }
}

static void gnoblin_control_prompt_get_property(GObject* object, guint property_id, GValue* value,
                                                GParamSpec* pspec) {
    GnoblinControlPrompt* self = GNOBLIN_CONTROL_PROMPT(object);
    switch (property_id) {
    case PROP_TITLE:
        g_value_set_string(value, self->title);
        break;
    case PROP_MESSAGE:
        g_value_set_string(value, self->message);
        break;
    case PROP_DESCRIPTION:
        g_value_set_string(value, self->description);
        break;
    case PROP_WARNING:
        g_value_set_string(value, self->warning);
        break;
    case PROP_CHOICE_LABEL:
        g_value_set_string(value, self->choice_label);
        break;
    case PROP_CHOICE_CHOSEN:
        g_value_set_boolean(value, self->choice_chosen);
        break;
    case PROP_PASSWORD_NEW:
        g_value_set_boolean(value, self->password_new);
        break;
    case PROP_PASSWORD_STRENGTH:
        /* Gnoblin does not rate passwords. */
        g_value_set_int(value, 0);
        break;
    case PROP_CALLER_WINDOW:
        g_value_set_string(value, self->caller_window);
        break;
    case PROP_CONTINUE_LABEL:
        g_value_set_string(value, self->continue_label);
        break;
    case PROP_CANCEL_LABEL:
        g_value_set_string(value, self->cancel_label);
        break;
    default:
        G_OBJECT_WARN_INVALID_PROPERTY_ID(object, property_id, pspec);
    }
}

static void gnoblin_control_prompt_finalize(GObject* object) {
    GnoblinControlPrompt* self = GNOBLIN_CONTROL_PROMPT(object);
    prompt_wipe_password(self);
    g_free(self->title);
    g_free(self->message);
    g_free(self->description);
    g_free(self->warning);
    g_free(self->choice_label);
    g_free(self->caller_window);
    g_free(self->continue_label);
    g_free(self->cancel_label);
    G_OBJECT_CLASS(gnoblin_control_prompt_parent_class)->finalize(object);
}

static void gnoblin_control_prompt_class_init(GnoblinControlPromptClass* klass) {
    GObjectClass* object_class = G_OBJECT_CLASS(klass);
    object_class->set_property = gnoblin_control_prompt_set_property;
    object_class->get_property = gnoblin_control_prompt_get_property;
    object_class->finalize = gnoblin_control_prompt_finalize;
    g_object_class_override_property(object_class, PROP_TITLE, "title");
    g_object_class_override_property(object_class, PROP_MESSAGE, "message");
    g_object_class_override_property(object_class, PROP_DESCRIPTION, "description");
    g_object_class_override_property(object_class, PROP_WARNING, "warning");
    g_object_class_override_property(object_class, PROP_CHOICE_LABEL, "choice-label");
    g_object_class_override_property(object_class, PROP_CHOICE_CHOSEN, "choice-chosen");
    g_object_class_override_property(object_class, PROP_PASSWORD_NEW, "password-new");
    g_object_class_override_property(object_class, PROP_PASSWORD_STRENGTH, "password-strength");
    g_object_class_override_property(object_class, PROP_CALLER_WINDOW, "caller-window");
    g_object_class_override_property(object_class, PROP_CONTINUE_LABEL, "continue-label");
    g_object_class_override_property(object_class, PROP_CANCEL_LABEL, "cancel-label");
}

static void gnoblin_control_prompt_init(GnoblinControlPrompt* self) {
    (void)self;
}

static const char* result_name_or_empty(const char* text) {
    return text ? text : "";
}

static const char* prompt_kind_name(PromptKind kind) {
    return kind == PROMPT_KIND_PASSWORD ? "password" : "confirm";
}

static void pending_prompt_publish_finished(PendingPrompt* pending) {
    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "request_id",
                          g_variant_new_int64((gint64)pending->request_id));
    g_variant_builder_add(&fields, "{sv}", "result", g_variant_new_string(pending->result));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    gnoblin_control_publish_event(pending->control, "gnoblin.prompt.finished", payload);
}

static void pending_prompt_free(gpointer user_data) {
    PendingPrompt* pending = user_data;
    if (pending->timeout_source_id)
        g_source_remove(pending->timeout_source_id);
    pending->timeout_source_id = 0;
    if (pending->cancellable && pending->cancelled_handler_id)
        g_cancellable_disconnect(pending->cancellable, pending->cancelled_handler_id);
    pending->cancelled_handler_id = 0;
    /* A request that nobody answered ends as cancelled. */
    if (pending->task) {
        g_autoptr(GTask) task = g_steal_pointer(&pending->task);
        if (pending->kind == PROMPT_KIND_PASSWORD)
            g_task_return_boolean(task, FALSE);
        else
            g_task_return_int(task, GCR_PROMPT_REPLY_CANCEL);
    }
    pending_prompt_publish_finished(pending);
    g_clear_object(&pending->prompt);
    g_clear_object(&pending->cancellable);
    g_clear_pointer(&pending->recipient_client_ids, g_hash_table_unref);
    g_free(pending);
}

static void pending_prompt_end(PendingPrompt* pending, const char* result) {
    GnoblinNativeControl* control = pending->control;
    gint64 request_id = (gint64)pending->request_id;
    pending->result = result;
    if (control->pending_prompts)
        g_hash_table_remove(control->pending_prompts, &request_id);
}

static gboolean pending_prompt_timeout(gpointer user_data) {
    PendingPrompt* pending = user_data;
    pending->timeout_source_id = 0;
    pending_prompt_end(pending, "timeout");
    return G_SOURCE_REMOVE;
}

static void pending_prompt_cancelled(GCancellable* cancellable, gpointer user_data) {
    PendingPrompt* pending = user_data;
    (void)cancellable;
    /* The handler is running, so it must not disconnect itself. */
    pending->cancelled_handler_id = 0;
    pending_prompt_end(pending, "cancelled");
}

static void prompt_start(GcrPrompt* prompt, PromptKind kind, GCancellable* cancellable,
                         GAsyncReadyCallback callback, gpointer user_data) {
    GnoblinControlPrompt* self = GNOBLIN_CONTROL_PROMPT(prompt);
    GnoblinNativeControl* control = active_control;
    g_autoptr(GTask) task = g_task_new(prompt, cancellable, callback, user_data);
    prompt_wipe_password(self);

    if (!control || control->stopping || !control->pending_prompts ||
        !control->supervised_runtime || control->runtime_worker_suspended ||
        !control->runtime_hello_sent ||
        g_hash_table_size(control->pending_prompts) >= MAX_PENDING_PROMPTS ||
        (cancellable && g_cancellable_is_cancelled(cancellable))) {
        if (kind == PROMPT_KIND_PASSWORD)
            g_task_return_boolean(task, FALSE);
        else
            g_task_return_int(task, GCR_PROMPT_REPLY_CANCEL);
        return;
    }

    PendingPrompt* pending = g_new0(PendingPrompt, 1);
    pending->control = control;
    pending->task = g_steal_pointer(&task);
    pending->prompt = G_OBJECT(g_object_ref(prompt));
    pending->kind = kind;
    pending->result = "cancelled";
    pending->request_id = ++control->next_prompt_request_id;
    pending->recipient_client_ids =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL);
    guint64* key = g_new(guint64, 1);
    *key = pending->request_id;
    g_hash_table_insert(control->pending_prompts, key, pending);
    pending->timeout_source_id =
        g_timeout_add_seconds(PROMPT_ANSWER_TIMEOUT_SECONDS, pending_prompt_timeout, pending);
    if (cancellable) {
        pending->cancellable = g_object_ref(cancellable);
        pending->cancelled_handler_id =
            g_cancellable_connect(cancellable, G_CALLBACK(pending_prompt_cancelled), pending, NULL);
    }

    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "request_id",
                          g_variant_new_int64((gint64)pending->request_id));
    g_variant_builder_add(&fields, "{sv}", "kind", g_variant_new_string(prompt_kind_name(kind)));
    g_variant_builder_add(&fields, "{sv}", "title",
                          g_variant_new_string(result_name_or_empty(self->title)));
    g_variant_builder_add(&fields, "{sv}", "message",
                          g_variant_new_string(result_name_or_empty(self->message)));
    g_variant_builder_add(&fields, "{sv}", "description",
                          g_variant_new_string(result_name_or_empty(self->description)));
    g_variant_builder_add(&fields, "{sv}", "warning",
                          g_variant_new_string(result_name_or_empty(self->warning)));
    g_variant_builder_add(&fields, "{sv}", "password_new",
                          g_variant_new_boolean(self->password_new));
    g_variant_builder_add(&fields, "{sv}", "choice_label",
                          g_variant_new_string(result_name_or_empty(self->choice_label)));
    g_variant_builder_add(&fields, "{sv}", "caller_window",
                          g_variant_new_string(result_name_or_empty(self->caller_window)));
    g_variant_builder_add(&fields, "{sv}", "continue_label",
                          g_variant_new_string(result_name_or_empty(self->continue_label)));
    g_variant_builder_add(&fields, "{sv}", "cancel_label",
                          g_variant_new_string(result_name_or_empty(self->cancel_label)));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    gnoblin_control_publish_event(control, "gnoblin.prompt.requested", payload);
}

static void prompt_password_async(GcrPrompt* prompt, GCancellable* cancellable,
                                  GAsyncReadyCallback callback, gpointer user_data) {
    prompt_start(prompt, PROMPT_KIND_PASSWORD, cancellable, callback, user_data);
}

static const gchar* prompt_password_finish(GcrPrompt* prompt, GAsyncResult* result,
                                           GError** error) {
    g_return_val_if_fail(g_task_is_valid(result, prompt), NULL);
    if (!g_task_propagate_boolean(G_TASK(result), error))
        return NULL;
    /* Valid until the prompt starts again, closes, or is destroyed. */
    return GNOBLIN_CONTROL_PROMPT(prompt)->password;
}

static void prompt_confirm_async(GcrPrompt* prompt, GCancellable* cancellable,
                                 GAsyncReadyCallback callback, gpointer user_data) {
    prompt_start(prompt, PROMPT_KIND_CONFIRM, cancellable, callback, user_data);
}

static GcrPromptReply prompt_confirm_finish(GcrPrompt* prompt, GAsyncResult* result,
                                            GError** error) {
    g_return_val_if_fail(g_task_is_valid(result, prompt), GCR_PROMPT_REPLY_CANCEL);
    gssize reply = g_task_propagate_int(G_TASK(result), error);
    return reply == GCR_PROMPT_REPLY_CONTINUE ? GCR_PROMPT_REPLY_CONTINUE
                                              : GCR_PROMPT_REPLY_CANCEL;
}

static void prompt_close(GcrPrompt* prompt) {
    GnoblinNativeControl* control = active_control;
    prompt_wipe_password(GNOBLIN_CONTROL_PROMPT(prompt));
    if (!control || !control->pending_prompts)
        return;
    /* The prompter closes a prompt that it no longer needs. End its pending request. */
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->pending_prompts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        PendingPrompt* pending = value;
        if (pending->prompt == G_OBJECT(prompt)) {
            pending_prompt_end(pending, "cancelled");
            return;
        }
    }
}

static void gnoblin_control_prompt_iface_init(GcrPromptInterface* iface) {
    iface->prompt_password_async = prompt_password_async;
    iface->prompt_password_finish = prompt_password_finish;
    iface->prompt_confirm_async = prompt_confirm_async;
    iface->prompt_confirm_finish = prompt_confirm_finish;
    iface->prompt_close = prompt_close;
}

/* Broker lifetime. */

static void broker_unref(GnoblinControlPromptBroker* broker) {
    if (--broker->ref_count > 0)
        return;
    g_clear_object(&broker->cancellable);
    g_clear_object(&broker->connection);
    g_clear_object(&broker->prompter);
    g_free(broker);
}

static void broker_name_user_data_free(gpointer user_data) {
    broker_unref(user_data);
}

const char* gnoblin_control_prompts_unavailable_reason(GnoblinNativeControl* control) {
    if (!control->prompt_broker)
        return "prompts_disabled";
    return control->prompt_broker->owned_names == 2 ? NULL : "name_unavailable";
}

gboolean gnoblin_control_prompts_available(GnoblinNativeControl* control) {
    return gnoblin_control_prompts_unavailable_reason(control) == NULL;
}

static void prompts_publish_capability_changed(GnoblinNativeControl* control) {
    if (control->stopping || !control->runtime_hello_sent)
        return;
    control->state_revision++;
    g_autoptr(GVariant) capabilities = gnoblin_control_capability_snapshot(control);
    gnoblin_control_publish_runtime_snapshot(control, "capabilities", capabilities,
                                             control->state_revision);
    const NativeCapability* changed = gnoblin_control_capability_by_id("prompt-broker");
    if (!changed)
        return;
    GVariantBuilder event;
    g_variant_builder_init(&event, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) capability = gnoblin_control_capability_snapshot_record(control, changed);
    g_variant_builder_add(&event, "{sv}", "capability", capability);
    g_variant_builder_add(&event, "{sv}", "revision",
                          g_variant_new_int64((gint64)control->state_revision));
    g_autoptr(GVariant) fields = g_variant_ref_sink(g_variant_builder_end(&event));
    gnoblin_control_publish_event(control, "gnoblin.capability.changed", fields);
}

static void broker_name_acquired(GDBusConnection* connection, const gchar* name,
                                 gpointer user_data) {
    GnoblinControlPromptBroker* broker = user_data;
    (void)connection;
    if (broker->stopped)
        return;
    broker->owned_names++;
    g_message("gnoblin-prompts: owning %s", name);
    if (broker->owned_names == 2)
        prompts_publish_capability_changed(broker->control);
}

static void broker_name_lost(GDBusConnection* connection, const gchar* name, gpointer user_data) {
    GnoblinControlPromptBroker* broker = user_data;
    (void)connection;
    if (broker->stopped)
        return;
    gboolean was_available = broker->owned_names == 2;
    g_message("gnoblin-prompts: could not own %s; stop gcr-prompter to use the prompt broker",
              name);
    if (broker->owned_names > 0 && was_available)
        broker->owned_names--;
    broker->name_failed = TRUE;
    if (was_available)
        prompts_publish_capability_changed(broker->control);
}

static void broker_bus_ready(GObject* source, GAsyncResult* result, gpointer user_data) {
    GnoblinControlPromptBroker* broker = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) connection = g_bus_get_finish(result, &error);
    if (broker->stopped || !connection) {
        if (!broker->stopped)
            g_message("gnoblin-prompts: session bus unavailable: %s",
                      error ? error->message : "unknown error");
        broker_unref(broker);
        return;
    }
    (void)source;
    broker->connection = g_object_ref(connection);
    gcr_system_prompter_register(broker->prompter, connection);
    broker->ref_count += 2;
    broker->system_name_id = g_bus_own_name_on_connection(
        connection, SYSTEM_PROMPTER_NAME, G_BUS_NAME_OWNER_FLAGS_DO_NOT_QUEUE,
        broker_name_acquired, broker_name_lost, broker, broker_name_user_data_free);
    broker->private_name_id = g_bus_own_name_on_connection(
        connection, PRIVATE_PROMPTER_NAME, G_BUS_NAME_OWNER_FLAGS_DO_NOT_QUEUE,
        broker_name_acquired, broker_name_lost, broker, broker_name_user_data_free);
    broker_unref(broker);
}

static void prompts_start(GnoblinNativeControl* control) {
    if (control->prompt_broker || control->stopping)
        return;
    GnoblinControlPromptBroker* broker = g_new0(GnoblinControlPromptBroker, 1);
    broker->ref_count = 2;
    broker->control = control;
    broker->cancellable = g_cancellable_new();
    /* gcr-prompter serves both names from one prompter object in single mode, because only
     * one object can answer on /org/gnome/keyring/Prompter. */
    broker->prompter = gcr_system_prompter_new(GCR_SYSTEM_PROMPTER_SINGLE,
                                               gnoblin_control_prompt_get_type());
    control->prompt_broker = broker;
    active_control = control;
    g_bus_get(G_BUS_TYPE_SESSION, broker->cancellable, broker_bus_ready, broker);
    prompts_publish_capability_changed(control);
}

static void prompts_shutdown(GnoblinNativeControl* control) {
    GnoblinControlPromptBroker* broker = control->prompt_broker;
    if (!broker)
        return;
    broker->stopped = TRUE;
    control->prompt_broker = NULL;
    active_control = NULL;
    g_cancellable_cancel(broker->cancellable);
    if (control->pending_prompts)
        g_hash_table_remove_all(control->pending_prompts);
    if (broker->system_name_id)
        g_bus_unown_name(broker->system_name_id);
    if (broker->private_name_id)
        g_bus_unown_name(broker->private_name_id);
    if (broker->connection)
        gcr_system_prompter_unregister(broker->prompter, FALSE);
    broker->system_name_id = 0;
    broker->private_name_id = 0;
    broker_unref(broker);
}

void gnoblin_control_prompts_init(GnoblinNativeControl* control) {
    control->pending_prompts =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, pending_prompt_free);
}

void gnoblin_control_prompts_apply_config(GnoblinNativeControl* control, GVariant* config) {
    if (!control || control->stopping)
        return;
    g_autoptr(GVariant) prompts =
        config ? g_variant_lookup_value(config, "prompts", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) enabled_value =
        prompts ? g_variant_lookup_value(prompts, "enabled", G_VARIANT_TYPE_BOOLEAN) : NULL;
    gboolean enabled = enabled_value && g_variant_get_boolean(enabled_value);
    if (enabled && !control->prompt_broker) {
        prompts_start(control);
    } else if (!enabled && control->prompt_broker) {
        prompts_shutdown(control);
        prompts_publish_capability_changed(control);
    }
}

void gnoblin_control_prompts_stop(GnoblinNativeControl* control) {
    prompts_shutdown(control);
    if (control->pending_prompts)
        g_hash_table_remove_all(control->pending_prompts);
}

void gnoblin_control_prompts_note_recipient(GnoblinNativeControl* control, gint64 request_id,
                                            guint64 client_id) {
    PendingPrompt* pending =
        request_id > 0 && control->pending_prompts
            ? g_hash_table_lookup(control->pending_prompts, &request_id)
            : NULL;
    if (pending) {
        guint64* client_key = g_new(guint64, 1);
        *client_key = client_id;
        g_hash_table_add(pending->recipient_client_ids, client_key);
    }
}

static gboolean prompt_optional_boolean(GVariant* arguments, const char* name, gboolean* present,
                                        gboolean* value, const char* method, GError** error) {
    g_autoptr(GVariant) field = g_variant_lookup_value(arguments, name, NULL);
    *present = field != NULL;
    if (!field)
        return TRUE;
    if (!g_variant_is_of_type(field, G_VARIANT_TYPE_BOOLEAN)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "%s %s must be a boolean",
                    method, name);
        return FALSE;
    }
    *value = g_variant_get_boolean(field);
    return TRUE;
}

GVariant* gnoblin_control_prompts_operation(GnoblinNativeControl* control, const char* method,
                                            GVariant* arguments, guint64 client_id,
                                            GError** error) {
    gint64 request_id = 0;
    if (!g_variant_lookup(arguments, "request_id", "x", &request_id) || request_id <= 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "%s requires a positive request_id", method);
        return NULL;
    }
    PendingPrompt* pending =
        control->pending_prompts ? g_hash_table_lookup(control->pending_prompts, &request_id)
                                 : NULL;
    if (!pending) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "prompt request is no longer pending");
        return NULL;
    }
    if (client_id && !g_hash_table_contains(pending->recipient_client_ids, &client_id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "only a client that received the request can answer it");
        return NULL;
    }

    if (g_str_equal(method, "prompt.respond")) {
        GnoblinControlPrompt* prompt = GNOBLIN_CONTROL_PROMPT(pending->prompt);
        gboolean password_present = FALSE;
        gboolean confirmed_present = FALSE;
        gboolean choice_present = FALSE;
        gboolean confirmed = FALSE;
        gboolean choice = FALSE;
        const char* password = NULL;
        g_autoptr(GVariant) password_field = g_variant_lookup_value(arguments, "password", NULL);
        if (password_field) {
            gsize length = 0;
            password_present = TRUE;
            if (!g_variant_is_of_type(password_field, G_VARIANT_TYPE_STRING)) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                    "prompt.respond password must be a string");
                return NULL;
            }
            password = g_variant_get_string(password_field, &length);
            if (length > PROMPT_MAX_PASSWORD_BYTES || !g_utf8_validate(password, length, NULL)) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                    "prompt.respond password must be UTF-8 and at most 4096 bytes");
                return NULL;
            }
        }
        if (!prompt_optional_boolean(arguments, "confirmed", &confirmed_present, &confirmed,
                                     method, error) ||
            !prompt_optional_boolean(arguments, "choice", &choice_present, &choice, method,
                                     error))
            return NULL;
        if (pending->kind == PROMPT_KIND_PASSWORD && (!password_present || confirmed_present)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "a password prompt needs prompt.respond password and no "
                                "confirmed");
            return NULL;
        }
        if (pending->kind == PROMPT_KIND_CONFIRM && (!confirmed_present || password_present)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "a confirm prompt needs prompt.respond confirmed and no password");
            return NULL;
        }

        if (choice_present)
            prompt->choice_chosen = choice;
        g_autoptr(GTask) task = g_steal_pointer(&pending->task);
        if (pending->kind == PROMPT_KIND_PASSWORD) {
            prompt_wipe_password(prompt);
            prompt->password = gcr_secure_memory_strdup(password);
            g_task_return_boolean(task, TRUE);
        } else {
            g_task_return_int(task, confirmed ? GCR_PROMPT_REPLY_CONTINUE
                                              : GCR_PROMPT_REPLY_CANCEL);
        }
        pending_prompt_end(pending, "answered");
    } else {
        /* prompt.cancel ends the request, which also answers the prompter with a cancel. */
        pending_prompt_end(pending, "cancelled");
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&result, "{sv}", "submitted", g_variant_new_boolean(TRUE));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}
