/*
 * gnoblin-shell: config-driven animation settings (per-effect duration +
 * curve).
 *
 * Each effect ("open", "close", "resize") reads its timing and easing from the
 * config's [animations] section, e.g.
 *
 *     [animations]
 *     enabled = on
 *     open  = 150, ease-out-cubic, 0.985  # duration (ms), curve, start scale
 *     close = 110, ease-in-cubic, 0.985
 *     open.dialog = 105, ease-out-cubic, 0.985
 *     open.utility = 105, ease-out-cubic, 0.985
 *     open.menu = 80, ease-out-quad, 0.995
 *     open.popup-menu = 80, ease-out-quad, 0.995
 *     resize = 160, ease-out-quint
 *     maximize = 260, ease-in-out-cubic
 *     unmaximize = 240, ease-in-out-cubic
 *
 * Window-type keys follow Mutter window type names. Menu-like types are used
 * for open effects only; destroy effects stay on stable toplevels in the shell
 * plugin because popup actors can disappear synchronously during protocol
 * validation. Size-change effects (`maximize`, `unmaximize`, `resize`) can be
 * refined the same way, e.g. `maximize.dialog`.
 *
 * Curve names map to Clutter easing modes (linear, ease-{in,out,in-out}, and
 * ease-{in,out,in-out}-{quad,cubic,quart,quint,sine,expo,circ,back,bounce,
 * elastic} — back/elastic/bounce give overshoot/spring). Following the motion
 * principles: entrances ease-out, exits ease-in, durations under 300ms.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#pragma once

extern "C" {
#include <clutter/clutter.h>
#include <meta/window.h>
}

typedef struct {
    gboolean enabled;
    guint duration_ms;
    ClutterAnimationMode mode;
    double scale;
} GnoblinAnim;

/* Resolve an effect's animation settings from the live config. */
GnoblinAnim gnoblin_anim_get(const char* effect);
GnoblinAnim gnoblin_anim_get_for_window(const char* effect, MetaWindow* window);
