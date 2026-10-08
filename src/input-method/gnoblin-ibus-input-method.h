/* IBus-backed input method for the standalone Gnoblin compositor. */

#pragma once

#include "meta/common.h"
#include "meta/meta-backend.h"

G_BEGIN_DECLS

/*
 * Install an input method that talks to ibus-daemon, if the backend has none.
 *
 * The input method connects to the daemon when it is running and reconnects when it
 * starts later. Without a daemon, or without an active engine, every key is passed
 * through untouched. Text-input-v3 clients get preedit and commit from it.
 */
META_EXPORT
void gnoblin_ibus_input_method_install (MetaBackend *backend);

G_END_DECLS
