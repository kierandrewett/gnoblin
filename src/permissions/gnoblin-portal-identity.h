/* SPDX-License-Identifier: LGPL-2.1-or-later */
#pragma once
#include <gio/gio.h>

/*
 * Verify that backend_sender owns org.freedesktop.portal.Desktop, recover the
 * original caller from request_handle, and return a namespaced identity. The
 * result is "app-id:<id>" when the trusted portal supplied an app id, or
 * "host-exe:<canonical-path>" for a live unsandboxed caller. Any failed check
 * returns NULL so policy cannot automatically authorise the caller.
 */
char *gnoblin_portal_requester_identity (GDBusConnection *connection,
                                         const char      *backend_sender,
                                         const char      *app_id,
                                         const char      *request_handle);

/* Return the readable part of a verified namespaced identity. */
const char *gnoblin_portal_identity_name (const char *identity);

