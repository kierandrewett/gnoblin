// SPDX-License-Identifier: GPL-2.0-or-later
#include "config.h"

extern "C" {
#include "backends/gnoblin-hyprcursor.h"
}

#include <hyprcursor/hyprcursor.hpp>
#include <cstring>
#include <memory>
#include <set>
#include <string>

namespace {
std::unique_ptr<Hyprcursor::CHyprcursorManager> manager;
std::string loaded_theme;
std::set<int> loaded_sizes;

struct ImagesDeleter {
    void operator()(XcursorImages* images) const {
        if (images)
            xcursor_images_destroy(images);
    }
};
} // namespace

void gnoblin_hyprcursor_invalidate(void) {
    loaded_sizes.clear();
    manager.reset();
    loaded_theme.clear();
}

XcursorImages* gnoblin_hyprcursor_load(const char* theme, const char* shape, int size) {
    if (g_strcmp0(g_getenv("GNOME_SHELL_SESSION_MODE"), "gnoblin") != 0 || !shape || size < 1 ||
        size > 1024)
        return nullptr;

    const char* override_theme = g_getenv("HYPRCURSOR_THEME");
    const std::string requested = override_theme && *override_theme ? override_theme
                                  : theme                           ? theme
                                                                    : "";
    if (requested.empty())
        return nullptr;

    try {
        if (!manager || loaded_theme != requested) {
            gnoblin_hyprcursor_invalidate();
            Hyprcursor::SManagerOptions options;
            // A missing theme must retain GNOME's fallback, not pick a random
            // Hyprcursor theme from the library's search paths.
            options.allowDefaultFallback = false;
            manager = std::make_unique<Hyprcursor::CHyprcursorManager>(requested.c_str(), options);
            loaded_theme = requested;
        }
        if (!manager->valid())
            return nullptr;

        const Hyprcursor::SCursorStyleInfo style{static_cast<unsigned int>(size)};
        if (!loaded_sizes.contains(size)) {
            if (!manager->loadThemeStyle(style))
                return nullptr;
            loaded_sizes.insert(size);
        }
        const auto data = manager->getShape(shape, style);
        if (data.images.empty() || data.images.size() > 1024)
            return nullptr;

        std::unique_ptr<XcursorImages, ImagesDeleter> images(
            xcursor_images_create(data.images.size()));
        if (!images)
            return nullptr;
        for (size_t i = 0; i < data.images.size(); i++) {
            const auto& frame = data.images[i];
            if (!frame.surface || cairo_surface_status(frame.surface) != CAIRO_STATUS_SUCCESS ||
                cairo_surface_get_type(frame.surface) != CAIRO_SURFACE_TYPE_IMAGE ||
                cairo_image_surface_get_format(frame.surface) != CAIRO_FORMAT_ARGB32)
                return nullptr;
            const int width = cairo_image_surface_get_width(frame.surface);
            const int height = cairo_image_surface_get_height(frame.surface);
            if (width < 1 || height < 1 || width > 4096 || height > 4096 || frame.hotspotX < 0 ||
                frame.hotspotX >= width || frame.hotspotY < 0 || frame.hotspotY >= height)
                return nullptr;
            XcursorImage* image = xcursor_image_create(width, height);
            if (!image)
                return nullptr;
            images->images[i] = image;
            images->nimage++;
            image->size = size;
            image->xhot = frame.hotspotX;
            image->yhot = frame.hotspotY;
            image->delay = MAX(1, frame.delay);
            cairo_surface_flush(frame.surface);
            const auto* pixels = cairo_image_surface_get_data(frame.surface);
            const int stride = cairo_image_surface_get_stride(frame.surface);
            for (int row = 0; row < height; row++)
                std::memcpy(image->pixels + row * width, pixels + row * stride,
                            width * sizeof(XcursorPixel));
        }
        return images.release();
    } catch (const std::exception& error) {
        g_debug("Hyprcursor theme %s failed: %s", requested.c_str(), error.what());
        gnoblin_hyprcursor_invalidate();
        return nullptr;
    }
}
