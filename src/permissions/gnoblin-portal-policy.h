/* SPDX-License-Identifier: LGPL-2.1-or-later */
#pragma once
#include <gio/gio.h>

typedef enum {
    GNOBLIN_PERMISSION_DEFAULT,
    GNOBLIN_PERMISSION_ASK,
    GNOBLIN_PERMISSION_ALLOW,
    GNOBLIN_PERMISSION_DENY,
} GnoblinPermissionLevel;

typedef struct {
    GnoblinPermissionLevel level;
    char** monitors;
    guint32 devices;
    gboolean clipboard;
    char* rule;
} GnoblinPermission;

static inline void gnoblin_permission_clear(GnoblinPermission* permission) {
    g_clear_pointer(&permission->monitors, g_strfreev);
    g_clear_pointer(&permission->rule, g_free);
}
G_DEFINE_AUTO_CLEANUP_CLEAR_FUNC(GnoblinPermission, gnoblin_permission_clear)

/* Identity must come from gnoblin_portal_requester_identity(), not app text. */
gboolean gnoblin_permission_policy_validate(GVariant* document, GError** error);
gboolean gnoblin_permission_capability_supported(const char* capability);
GnoblinPermission gnoblin_permission_policy_evaluate(GVariant* document, const char* capability,
                                                     const char* identity);
GVariant* gnoblin_permission_policy_list(GVariant* document, const char* config_path);
/* The public policy value without compatibility metadata, with its committed
 * configuration revision attached. */
GVariant* gnoblin_permission_policy_snapshot(GVariant* document, guint64 revision);
GnoblinPermission gnoblin_permission_check(GDBusMethodInvocation* invocation,
                                           const char* capability, const char* identity);
GnoblinPermission gnoblin_permission_for_request(GDBusMethodInvocation* invocation,
                                                 const char* capability, const char* app_id,
                                                 const char* handle);
