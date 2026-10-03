#pragma once

#include "../native-control/gnoblin-runtime-protocol.h"

G_BEGIN_DECLS

/* Receives the one-shot host autostart packet. The packet contains exactly
 * `entries` (av) and `environment` (a{ss}); EOF is valid only after receipt. */
gboolean gnoblin_autostart_receive_packet(int fd, GnoblinRuntimeReader* reader,
                                          gboolean* received_packet, GVariant** entries,
                                          GVariant** environment, GError** error);

/* Build an inherited environment with session display variables replaced by
 * the validated compositor environment. Free the result with g_strfreev(). */
char** gnoblin_autostart_build_environment(GVariant* environment, GError** error);

G_END_DECLS
