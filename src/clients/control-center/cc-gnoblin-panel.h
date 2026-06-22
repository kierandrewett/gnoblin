/* cc-gnoblin-panel.h
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <shell/cc-panel.h>

G_BEGIN_DECLS

#define CC_TYPE_GNOBLIN_PANEL (cc_gnoblin_panel_get_type())
G_DECLARE_FINAL_TYPE (CcGnoblinPanel, cc_gnoblin_panel, CC, GNOBLIN_PANEL, CcPanel)

G_END_DECLS
