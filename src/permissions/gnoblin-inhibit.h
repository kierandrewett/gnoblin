// SPDX-License-Identifier: LGPL-2.1-or-later
#pragma once

#include <gio/gio.h>

gboolean gnoblin_inhibit_init(GDBusConnection* connection, GError** error);
