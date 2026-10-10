// SPDX-License-Identifier: LGPL-2.1-or-later

#pragma once

#include <gio/gio.h>

gboolean email_init(GDBusConnection* bus, GError** error);
