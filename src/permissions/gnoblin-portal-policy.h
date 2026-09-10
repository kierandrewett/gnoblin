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
  char **monitors;
  guint32 devices;
  gboolean clipboard;
} GnoblinPermission;

static inline void
gnoblin_permission_clear (GnoblinPermission *permission)
{
  g_clear_pointer (&permission->monitors, g_strfreev);
}
G_DEFINE_AUTO_CLEANUP_CLEAR_FUNC (GnoblinPermission, gnoblin_permission_clear)

/* Identity must come from gnoblin_portal_requester_identity(), not app text. */
GnoblinPermission gnoblin_permission_check (GDBusMethodInvocation *invocation,
                                            const char *capability,
                                            const char *identity);
GnoblinPermission gnoblin_permission_for_request (GDBusMethodInvocation *invocation,
                                                  const char *capability,
                                                  const char *app_id,
                                                  const char *handle);
