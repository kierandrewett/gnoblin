// SPDX-License-Identifier: LGPL-2.1-or-later
// Launch the configured mailto handler for the Email portal.

#include "config.h"

#include <gio/gio.h>

#include "gnoblin-email.h"
#include "xdg-desktop-portal-dbus.h"
#include "utils.h"

static void append_address(GString* uri, const char* address, const char* separator) {
    g_autofree char* escaped = g_uri_escape_string(address, "@._+-", FALSE);
    g_string_append(uri, separator);
    g_string_append(uri, escaped);
}

static void append_address_list(GString* uri, const char* key, char** addresses,
                                gboolean* has_query) {
    if (!addresses || !addresses[0])
        return;

    g_string_append_printf(uri, "%c%s=", *has_query ? '&' : '?', key);
    *has_query = TRUE;
    for (guint i = 0; addresses[i]; i++)
        append_address(uri, addresses[i], i == 0 ? "" : ",");
}

static void append_query(GString* uri, const char* key, const char* value, gboolean* has_query) {
    if (!value)
        return;

    g_autofree char* escaped = g_uri_escape_string(value, NULL, FALSE);
    g_string_append_printf(uri, "%c%s=%s", *has_query ? '&' : '?', key, escaped);
    *has_query = TRUE;
}

static gboolean handle_compose_email(XdpImplEmail* impl, GDBusMethodInvocation* invocation,
                                     const char* handle, const char* app_id,
                                     const char* parent_window, GVariant* options) {
    const char* address = NULL;
    const char* subject = NULL;
    const char* body = NULL;
    const char* token = NULL;
    g_auto(GStrv) addresses = NULL;
    g_auto(GStrv) cc = NULL;
    g_auto(GStrv) bcc = NULL;
    g_auto(GStrv) attachments = NULL;
    g_autoptr(GString) uri = g_string_new("mailto:");
    g_autoptr(GAppInfo) handler = NULL;
    g_autoptr(GAppLaunchContext) context = NULL;
    g_autoptr(GError) error = NULL;
    gboolean has_query = FALSE;
    guint response = 2;

    g_variant_lookup(options, "address", "&s", &address);
    g_variant_lookup(options, "addresses", "^as", &addresses);
    g_variant_lookup(options, "cc", "^as", &cc);
    g_variant_lookup(options, "bcc", "^as", &bcc);
    g_variant_lookup(options, "subject", "&s", &subject);
    g_variant_lookup(options, "body", "&s", &body);
    g_variant_lookup(options, "attachments", "^as", &attachments);
    g_variant_lookup(options, "activation_token", "&s", &token);

    if (address && *address)
        append_address(uri, address, "");
    for (guint i = 0; addresses && addresses[i]; i++)
        append_address(uri, addresses[i], uri->len == sizeof("mailto:") - 1 ? "" : ",");
    append_address_list(uri, "cc", cc, &has_query);
    append_address_list(uri, "bcc", bcc, &has_query);
    append_query(uri, "subject", subject, &has_query);
    append_query(uri, "body", body, &has_query);
    for (guint i = 0; attachments && attachments[i]; i++) {
        g_autoptr(GFile) file = g_file_new_for_commandline_arg(attachments[i]);
        g_autofree char* file_uri = g_file_get_uri(file);
        append_query(uri, "attachment", file_uri, &has_query);
    }

    handler = g_app_info_get_default_for_uri_scheme("mailto");
    if (handler) {
        context = g_app_launch_context_new();
        if (token && *token)
            g_app_launch_context_setenv(context, "XDG_ACTIVATION_TOKEN", token);
        GList* uris = g_list_append(NULL, uri->str);
        if (g_app_info_launch_uris(handler, uris, context, &error))
            response = 0;
        else
            g_warning("Could not launch mailto handler: %s", error->message);
        g_list_free(uris);
    }

    GVariantBuilder results;
    g_variant_builder_init(&results, G_VARIANT_TYPE_VARDICT);
    xdp_impl_email_complete_compose_email(impl, invocation, response,
                                          g_variant_builder_end(&results));
    return TRUE;
}

gboolean email_init(GDBusConnection* bus, GError** error) {
    GDBusInterfaceSkeleton* helper = G_DBUS_INTERFACE_SKELETON(xdp_impl_email_skeleton_new());
    g_signal_connect(helper, "handle-compose-email", G_CALLBACK(handle_compose_email), NULL);
    if (!g_dbus_interface_skeleton_export(helper, bus, DESKTOP_PORTAL_OBJECT_PATH, error)) {
        g_object_unref(helper);
        return FALSE;
    }
    g_debug("providing %s", g_dbus_interface_skeleton_get_info(helper)->name);
    return TRUE;
}
