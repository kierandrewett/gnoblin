/* cc-gnoblin-panel.h
 *
 * gnoblin overlay panel for gnome-control-center. Drives the org.gnoblin.Shell
 * control protocol (feature toggles + screencast grants + soft reload). This
 * file is gnoblin overlay source (src/control-center/), copied verbatim into the
 * gnome-control-center submodule by scripts/copy-overlay.sh; the only upstream
 * edits are a one-line panel-list entry in shell/cc-panel-loader.c and a one-line
 * 'gnoblin' entry in panels/meson.build (patches/gnome-control-center/).
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include "cc-panel.h"

G_BEGIN_DECLS

#define CC_TYPE_GNOBLIN_PANEL (cc_gnoblin_panel_get_type ())
G_DECLARE_FINAL_TYPE (CcGnoblinPanel, cc_gnoblin_panel, CC, GNOBLIN_PANEL, CcPanel)

G_END_DECLS
