#define _GNU_SOURCE
#include <dlfcn.h>
#include <glib.h>
#include <stdlib.h>
#include <unistd.h>
typedef gboolean (*Blit)(void*, void*, int, int, int, int, int, int, GError**);
gboolean cogl_framebuffer_blit(void* src, void* dst, int sx, int sy, int dx, int dy, int w, int h,
                               GError** error) {
    const char* marker = getenv("GNOBLIN_TEST_BLUR_BLIT_FAIL");
    if (marker && access(marker, F_OK) == 0) {
        g_set_error_literal(error, g_quark_from_static_string("gnoblin-blur-fixture"), 1,
                            "injected backdrop capture failure");
        return FALSE;
    }
    Blit next = (Blit)dlsym(RTLD_NEXT, "cogl_framebuffer_blit");
    return next(src, dst, sx, sy, dx, dy, w, h, error);
}
