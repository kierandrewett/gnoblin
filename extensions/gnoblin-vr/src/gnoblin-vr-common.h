#pragma once

/* Shared OpenXR client helpers for the Gnoblin VR extension binary. */

#ifndef XR_USE_GRAPHICS_API_OPENGL
#define XR_USE_GRAPHICS_API_OPENGL 1
#endif
#ifndef XR_USE_PLATFORM_XLIB
#define XR_USE_PLATFORM_XLIB 1
#endif
#ifndef GL_GLEXT_PROTOTYPES
#define GL_GLEXT_PROTOTYPES 1
#endif
#include <X11/Xlib.h>
#include <GL/glx.h>
#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>
#include <glib.h>
#include <signal.h>
#include <sys/stat.h>
#include <unistd.h>
#include "gnoblin-vr-status.h"

extern volatile gboolean gnoblin_vr_stopping;
void gnoblin_vr_handle_stop(int signal_number);
void gnoblin_vr_write_terminal_status(const char* state, const char* diagnostic, guint count,
                                      const guint32* widths, const guint32* heights);
void gnoblin_vr_select_monado_service(void);
gboolean gnoblin_vr_extension_available(const char* extension);
int gnoblin_vr_absent(const char* diagnostic);
int gnoblin_vr_failed(const char* diagnostic);
/* Runs the GL projection-layer session. Returns a process exit style code and never
 * returns while rendering except on stop/disconnect. */
int gnoblin_vr_gl_main(void);
