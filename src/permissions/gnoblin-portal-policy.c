/* SPDX-License-Identifier: LGPL-2.1-or-later */
#include "gnoblin-portal-policy.h"
#include "gnoblin-portal-identity.h"

static gboolean
is_gnoblin_session (void)
{
  const char *desktop = g_getenv ("XDG_CURRENT_DESKTOP");
  g_auto(GStrv) desktops = g_strsplit (desktop ? desktop : "", ":", -1);
  for (guint i = 0; desktops[i]; i++)
    if (g_ascii_strcasecmp (desktops[i], "gnoblin") == 0)
      return TRUE;
  return g_strcmp0 (g_getenv ("GNOME_SHELL_SESSION_MODE"), "gnoblin") == 0;
}

GnoblinPermission
gnoblin_permission_check (GDBusMethodInvocation *invocation,
                           const char *capability,
                           const char *identity)
{
  GnoblinPermission permission = { .level = GNOBLIN_PERMISSION_DEFAULT };
  g_autoptr(GVariant) reply = NULL;
  g_autoptr(GError) error = NULL;
  const char *level, *rule;

  if (!is_gnoblin_session ())
    return permission;

  /* A missing policy service must never restore a previous approval. */
  permission.level = GNOBLIN_PERMISSION_DENY;
  reply = g_dbus_connection_call_sync (
    g_dbus_method_invocation_get_connection (invocation),
    "org.gnoblin.Shell", "/org/gnoblin/Shell", "org.gnoblin.Shell",
    "CheckPermission", g_variant_new ("(ss)", capability, identity ? identity : ""),
    G_VARIANT_TYPE ("(ssasub)"), G_DBUS_CALL_FLAGS_NO_AUTO_START, 2000, NULL, &error);
  if (!reply)
    {
      g_warning ("gnoblin: permission policy unavailable: %s", error->message);
      return permission;
    }
  g_variant_get (reply, "(&s&s^asub)", &level, &rule, &permission.monitors,
                 &permission.devices, &permission.clipboard);
  if (g_str_equal (level, "default")) permission.level = GNOBLIN_PERMISSION_DEFAULT;
  else if (g_str_equal (level, "ask")) permission.level = GNOBLIN_PERMISSION_ASK;
  else if (g_str_equal (level, "allow") && identity) permission.level = GNOBLIN_PERMISSION_ALLOW;
  g_debug ("gnoblin: permission %s for %s: %s (rule %s)", capability,
           identity ? identity : "unverified", level, rule);
  return permission;
}

GnoblinPermission
gnoblin_permission_for_request (GDBusMethodInvocation *invocation,
                                 const char *capability,
                                 const char *app_id,
                                 const char *handle)
{
  g_autofree char *identity = gnoblin_portal_requester_identity (
    g_dbus_method_invocation_get_connection (invocation),
    g_dbus_method_invocation_get_sender (invocation), app_id, handle);
  return gnoblin_permission_check (invocation, capability, identity);
}
