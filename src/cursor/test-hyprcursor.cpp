#include "config.h"
extern "C" {
#include "backends/gnoblin-hyprcursor.h"
#include "core/gnoblin-native-control.h"
}
#include <cassert>
#include <cstdio>
#include <initializer_list>

namespace {
bool supervisor_session = true;
}

extern "C" gboolean gnoblin_native_control_is_session(MetaDisplay* display) {
    assert(display == nullptr);
    return supervisor_session;
}

int main() {
    g_unsetenv("GNOME_SHELL_SESSION_MODE");
    g_unsetenv("HYPRCURSOR_THEME");
    for (const int size : {24, 48, 72}) {
        XcursorImages* images = gnoblin_hyprcursor_load("GnoblinVectorTest", "default", size);
        assert(images && images->nimage == 2);
        for (int i = 0; i < 2; i++) {
            const XcursorImage* image = images->images[i];
            assert(image->width == static_cast<unsigned>(size) &&
                   image->height == static_cast<unsigned>(size));
            assert(image->xhot == static_cast<unsigned>(size / 4) &&
                   image->yhot == static_cast<unsigned>(size / 4));
            assert(image->delay == static_cast<unsigned>(i == 0 ? 35 : 70));
            assert(image->pixels[size * (size / 2) + size / 2] ==
                   (i == 0 ? 0xffff00ffu : 0xff00ffffu));
        }
        xcursor_images_destroy(images);
    }
    assert(!gnoblin_hyprcursor_load("GnoblinVectorTest", "missing-shape", 24));
    assert(!gnoblin_hyprcursor_load("__missing_theme__", "default", 24));
    assert(!gnoblin_hyprcursor_load("GnoblinVectorTest", "default", 0));
    gnoblin_hyprcursor_invalidate();
    XcursorImages* reloaded = gnoblin_hyprcursor_load("GnoblinVectorTest", "default", 48);
    assert(reloaded);
    xcursor_images_destroy(reloaded);
    supervisor_session = false;
    g_setenv("GNOME_SHELL_SESSION_MODE", "gnoblin", TRUE);
    assert(!gnoblin_hyprcursor_load("GnoblinVectorTest", "default", 24));
    gnoblin_hyprcursor_invalidate();
    std::puts("HYPRCURSOR_PASSED: SVG sizes, pixels, animation, hotspots, fallback, invalidation "
              "and stock session isolation");
}
