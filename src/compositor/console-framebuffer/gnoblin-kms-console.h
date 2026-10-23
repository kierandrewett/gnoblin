/* SPDX-License-Identifier: GPL-2.0-or-later */
#pragma once

#include <glib.h>
#include "meta/meta-backend.h"
#include "meta/types.h"

G_BEGIN_DECLS

/*
 * The console framebuffer: the picture the kernel puts back on the display when the last
 * compositor closes the DRM device. Until the next compositor presents its first frame the
 * display shows whatever that buffer holds, which is black or leftover console text. Painting
 * compositor.background into it keeps the wallpaper on screen across a login handover.
 */
typedef struct _GnoblinKmsConsole GnoblinKmsConsole;

/*
 * Return the pixels for one console framebuffer: width * height * 4 bytes, tightly packed, each
 * pixel the bytes B, G, R, unused. Returning NULL skips that framebuffer.
 */
typedef GBytes* (*GnoblinKmsConsoleRender)(int width, int height, gpointer user_data);

/*
 * Remember the framebuffers the displays scan out now. Call it before the first frame is
 * presented, while the kernel's console framebuffer is still on screen. Returns NULL when the
 * backend is not native, or when no display shows a framebuffer this module can write.
 */
META_EXPORT
GnoblinKmsConsole* gnoblin_kms_console_capture(MetaBackend* backend);

/* Paint every remembered framebuffer with the pixels @render returns for its size. */
META_EXPORT
void gnoblin_kms_console_paint(GnoblinKmsConsole* console, GnoblinKmsConsoleRender render,
                               gpointer user_data);

META_EXPORT
void gnoblin_kms_console_free(GnoblinKmsConsole* console);

G_END_DECLS
