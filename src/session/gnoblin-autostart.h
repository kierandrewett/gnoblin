#pragma once

#include "../native-control/gnoblin-runtime-protocol.h"

G_BEGIN_DECLS

/* Receives one or more host autostart snapshots. Each packet contains exactly
 * `entries` (av) and `environment` (a{ss}). A later packet supersedes the
 * previous snapshot and is used to reconcile Gnoblin-owned autostart children. */
gboolean gnoblin_autostart_receive_packet(int fd, GnoblinRuntimeReader* reader,
                                          gboolean* received_packet, GVariant** entries,
                                          GVariant** environment, GError** error);

/* Build an inherited environment with session display variables replaced by
 * the validated compositor environment. Free the result with g_strfreev(). */
char** gnoblin_autostart_build_environment(GVariant* environment, GError** error);

G_END_DECLS
