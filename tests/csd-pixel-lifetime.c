#include <cogl/cogl.h>
#include <gio/gio.h>
#include <stdint.h>
#include <string.h>

typedef struct {
    CoglContext parent;
} FixtureContext;
typedef CoglContextClass FixtureContextClass;
G_DEFINE_TYPE(FixtureContext, fixture_context, COGL_TYPE_CONTEXT)
static void fixture_context_class_init(FixtureContextClass* klass) {
    (void)klass;
}
static void fixture_context_init(FixtureContext* self) {
    (void)self;
}

/* Exercise the patched get_pixels function with real malloc-backed Cogl bitmaps.
 * Only texture/readback producers are substituted; no renderer is started. */
typedef struct {
    int width, height;
} FixtureTexture;
typedef struct {
    FixtureTexture* texture;
    int dst_width, dst_height;
    gboolean offscreen, padded, unavailable;
} MetaShapedTexture;

static CoglContext* context;
static guint direct_calls, fallback_calls, destroyed;
static const guint8* bitmap_pixels;
static gboolean fail_map;
static const guint8 expected[] = {0, 0, 0, 0, 17, 33, 65, 128, 19, 41, 83, 255, 1, 2, 3, 4};

static void bitmap_destroyed(gpointer data, GObject* object) {
    (void)data;
    (void)object;
    destroyed++;
}
static G_GNUC_UNUSED void meta_shaped_texture_ensure_size_valid(MetaShapedTexture* stex) {
    (void)stex;
}
static G_GNUC_UNUSED gboolean
meta_shaped_texture_should_get_via_offscreen(MetaShapedTexture* stex) {
    return stex->offscreen;
}
static G_GNUC_UNUSED CoglTexture* fixture_plane(FixtureTexture* texture, int plane) {
    g_assert_cmpint(plane, ==, 0);
    return (CoglTexture*)texture;
}
static G_GNUC_UNUSED int fixture_width(CoglTexture* texture) {
    return ((FixtureTexture*)texture)->width;
}
static G_GNUC_UNUSED int fixture_height(CoglTexture* texture) {
    return ((FixtureTexture*)texture)->height;
}
#define meta_multi_texture_get_plane fixture_plane
#define cogl_texture_get_width fixture_width
#define cogl_texture_get_height fixture_height

static CoglBitmap* make_bitmap(gboolean padded, CoglPixelFormat format) {
    g_assert_cmpint(format, ==, COGL_PIXEL_FORMAT_RGBA_8888_PRE);
    CoglBitmap* bitmap;
    if (padded) {
        guint8* pixels = g_malloc0(24);
        memcpy(pixels, expected, 8);
        memcpy(pixels + 12, expected + 8, 8);
        memset(pixels + 8, 0xee, 4);
        memset(pixels + 20, 0xee, 4);
        bitmap = cogl_bitmap_new_for_data(context, 2, 2, format, 12, pixels);
        g_object_set_data_full(G_OBJECT(bitmap), "fixture-pixels", pixels, g_free);
    } else {
        bitmap = cogl_bitmap_new_with_malloc_buffer(context, 2, 2, format, NULL);
        guint8* pixels = cogl_bitmap_map(bitmap, COGL_BUFFER_ACCESS_WRITE, 0, NULL);
        memcpy(pixels, expected, sizeof(expected));
        cogl_bitmap_unmap(bitmap);
    }
    bitmap_pixels = cogl_bitmap_map(bitmap, COGL_BUFFER_ACCESS_READ, 0, NULL);
    cogl_bitmap_unmap(bitmap);
    g_object_weak_ref(G_OBJECT(bitmap), bitmap_destroyed, NULL);
    return bitmap;
}
static CoglBitmap* meta_shaped_texture_paint_to_bitmap(MetaShapedTexture* stex, void* clip,
                                                       CoglPixelFormat format) {
    g_assert_null(clip);
    direct_calls++;
    if (!stex->texture || stex->offscreen || stex->unavailable)
        return NULL;
    if (stex->texture->width <= 0 || stex->texture->height <= 0 ||
        (gint64)stex->texture->width * stex->texture->height > 16000000)
        return NULL; /* Count the call without allocating an oversized fixture. */
    return make_bitmap(stex->padded, format);
}
static CoglBitmap* meta_shaped_texture_render_to_bitmap(MetaShapedTexture* stex,
                                                        CoglPixelFormat format, GError** error) {
    fallback_calls++;
    if (stex->dst_width <= 0 || stex->dst_height <= 0 ||
        (gint64)stex->dst_width * stex->dst_height > 16000000) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "fixture bounds");
        return NULL;
    }
    return make_bitmap(stex->padded, format);
}

static guint8* fixture_map(CoglBitmap* bitmap, CoglBufferAccess access, CoglBufferMapHint hints,
                           GError** error) {
    if (fail_map) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "fixture map failure");
        return NULL;
    }
    return cogl_bitmap_map(bitmap, access, hints, error);
}
#define cogl_bitmap_map fixture_map
#include "csd-get-pixels.inc"
#undef cogl_bitmap_map

static void reset(void) {
    direct_calls = fallback_calls = destroyed = 0;
    bitmap_pixels = NULL;
    fail_map = FALSE;
}
static void rejected_before_readback(void) {
    const int sizes[][5] = {{5000, 4000, 2, 2, 0}, {2, 2, 5000, 4000, 1},
                            {0, 4000, 0, 4000, 0}, {G_MAXINT, G_MAXINT, 2, 2, 0},
                            {2, 2, 0, 2, 1},       {-1, 4000, 2, 2, 0}};
    for (guint i = 0; i < G_N_ELEMENTS(sizes); i++) {
        reset();
        FixtureTexture texture = {sizes[i][0], sizes[i][1]};
        MetaShapedTexture stex = {.texture = &texture,
                                  .dst_width = sizes[i][2],
                                  .dst_height = sizes[i][3],
                                  .offscreen = sizes[i][4]};
        int width = -1, height = -1;
        g_autoptr(GError) error = NULL;
        g_autoptr(GBytes) bytes = meta_shaped_texture_get_pixels(&stex, &width, &height, &error);
        g_assert_cmpuint(direct_calls, ==, 0);
        g_assert_cmpuint(fallback_calls, ==, 0);
        g_assert_null(bytes);
        g_assert_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT);
        g_assert_cmpint(width, ==, 0);
        g_assert_cmpint(height, ==, 0);
    }
}
static void selected_path_dimensions(void) {
    const int sizes[][5] = {{4000, 4000, 2, 2, 0}, {2, 2, 5000, 4000, 0}, {5000, 4000, 2, 2, 1}};
    for (guint i = 0; i < G_N_ELEMENTS(sizes); i++) {
        reset();
        FixtureTexture texture = {sizes[i][0], sizes[i][1]};
        MetaShapedTexture stex = {.texture = &texture,
                                  .dst_width = sizes[i][2],
                                  .dst_height = sizes[i][3],
                                  .offscreen = sizes[i][4]};
        int width, height;
        g_autoptr(GError) error = NULL;
        g_autoptr(GBytes) bytes = meta_shaped_texture_get_pixels(&stex, &width, &height, &error);
        g_assert_no_error(error);
        g_assert_nonnull(bytes);
        g_assert_cmpuint(direct_calls, ==, 1);
        g_assert_cmpuint(fallback_calls, ==, stex.offscreen ? 1 : 0);
    }
}
static void failed_map_releases_bitmap(void) {
    reset();
    FixtureTexture texture = {2, 2};
    MetaShapedTexture stex = {.texture = &texture, .dst_width = 2, .dst_height = 2};
    int width, height;
    g_autoptr(GError) error = NULL;
    fail_map = TRUE;
    g_autoptr(GBytes) bytes = meta_shaped_texture_get_pixels(&stex, &width, &height, &error);
    g_assert_null(bytes);
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_FAILED);
    g_assert_cmpuint(destroyed, ==, 1);
}
static void pixel_identity(gconstpointer data) {
    guint mode = GPOINTER_TO_UINT(data);
    for (guint i = 0; i < 100; i++) {
        reset();
        FixtureTexture texture = {2, 2};
        MetaShapedTexture stex = {.texture = &texture,
                                  .dst_width = 2,
                                  .dst_height = 2,
                                  .offscreen = mode & 1,
                                  .padded = mode & 2,
                                  .unavailable = mode & 4};
        int width = -1, height = -1;
        g_autoptr(GError) error = NULL;
        GBytes* bytes = meta_shaped_texture_get_pixels(&stex, &width, &height, &error);
        g_assert_no_error(error);
        g_assert_nonnull(bytes);
        g_assert_cmpint(width, ==, 2);
        g_assert_cmpint(height, ==, 2);
        g_assert_cmpuint(direct_calls, ==, 1);
        g_assert_cmpuint(fallback_calls, ==, (mode & 5) ? 1 : 0);
        g_assert_cmpmem(g_bytes_get_data(bytes, NULL), g_bytes_get_size(bytes), expected,
                        sizeof(expected));
        if (stex.padded) {
            g_assert_true(g_bytes_get_data(bytes, NULL) != bitmap_pixels);
            g_assert_cmpuint(destroyed, ==, 1);
        } else {
            g_assert_true(g_bytes_get_data(bytes, NULL) == bitmap_pixels);
            g_assert_cmpuint(destroyed, ==, 0);
        }
        /* The readback bytes must outlive the source and an intermediate GBytes ref. */
        memset(&stex, 0, sizeof(stex));
        memset(&texture, 0, sizeof(texture));
        GBytes* retained = g_bytes_ref(bytes);
        g_bytes_unref(bytes);
        guint8* churn = g_malloc0(4096);
        g_assert_cmpmem(g_bytes_get_data(retained, NULL), g_bytes_get_size(retained), expected,
                        sizeof(expected));
        g_free(churn);
        if (!(mode & 2))
            g_assert_cmpuint(destroyed, ==, 0);
        g_bytes_unref(retained);
        g_assert_cmpuint(destroyed, ==, 1);
    }
}
int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    /* GObject initialization only: no display, renderer or GL context is realized. */
    context = g_object_new(fixture_context_get_type(), NULL);
    g_test_add_func("/csd/reject-before-readback", rejected_before_readback);
    g_test_add_func("/csd/selected-path-dimensions", selected_path_dimensions);
    g_test_add_func("/csd/map-failure-releases-bitmap", failed_map_releases_bitmap);
    g_test_add_data_func("/csd/direct-tight", GUINT_TO_POINTER(0), pixel_identity);
    g_test_add_data_func("/csd/fallback-tight", GUINT_TO_POINTER(1), pixel_identity);
    g_test_add_data_func("/csd/direct-padded", GUINT_TO_POINTER(2), pixel_identity);
    g_test_add_data_func("/csd/fallback-padded", GUINT_TO_POINTER(3), pixel_identity);
    g_test_add_data_func("/csd/unavailable-direct-tight", GUINT_TO_POINTER(4), pixel_identity);
    g_test_add_data_func("/csd/unavailable-direct-padded", GUINT_TO_POINTER(6), pixel_identity);
    int result = g_test_run();
    g_object_unref(context);
    return result;
}
