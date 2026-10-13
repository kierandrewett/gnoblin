#define _GNU_SOURCE
#include <dlfcn.h>
#include <glib.h>
#include <glib-object.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Deliberately opaque: use only exported GObject/Cogl/Clutter entry points. */
typedef struct _CoglFramebuffer CoglFramebuffer;
typedef struct _CoglContext CoglContext;
typedef struct _CoglTexture CoglTexture;
typedef struct _ClutterPaintNode ClutterPaintNode;
typedef struct _ClutterBlurCache ClutterBlurCache;
typedef struct _GError GError;
typedef ClutterPaintNode* (*NewCachedFn)(CoglFramebuffer*, int, int, unsigned int, unsigned int,
                                         float, float, float, unsigned char, ClutterBlurCache*);
typedef ClutterBlurCache* (*CacheNewFn)(void);
typedef unsigned int (*BuildCountFn)(ClutterBlurCache*);
typedef void (*NodeUnrefFn)(ClutterPaintNode*);
typedef CoglTexture* (*TextureNewFn)(CoglContext*, int, int);
typedef gboolean (*FramebufferAllocateFn)(CoglFramebuffer*, GError**);
typedef void (*ObjectUnrefFn)(void*);

static NewCachedFn real_new_cached;
static CacheNewFn cache_new;
static BuildCountFn build_count;
static NodeUnrefFn node_unref;
static TextureNewFn real_texture_new;
static FramebufferAllocateFn real_framebuffer_allocate;
static ObjectUnrefFn object_unref;
static GMutex lock;
static GHashTable* live_textures;
static guint weak_releases;
static gboolean armed, done;
static __thread gboolean running;
static const char* report_path;
static __thread guint fail_ordinal, allocation_ordinal, framebuffer_count;
static __thread CoglFramebuffer* framebuffers[32];
static __thread guint framebuffer_ordinals[32];
static __thread guint tracked_textures;
static __thread CoglTexture* textures[64];
static __thread guint texture_count;
static __thread int texture_widths[64], texture_heights[64];
static char error_text[512];

static void resolve_symbols(void) {
    real_new_cached =
        (NewCachedFn)dlsym(RTLD_NEXT, "clutter_blur_node_new_from_framebuffer_cached");
    cache_new = (CacheNewFn)dlsym(RTLD_NEXT, "clutter_blur_cache_new");
    build_count = (BuildCountFn)dlsym(RTLD_NEXT, "clutter_blur_cache_get_build_count");
    node_unref = (NodeUnrefFn)dlsym(RTLD_NEXT, "clutter_paint_node_unref");
    real_texture_new = (TextureNewFn)dlsym(RTLD_NEXT, "cogl_texture_2d_new_with_size");
    real_framebuffer_allocate =
        (FramebufferAllocateFn)dlsym(RTLD_NEXT, "cogl_framebuffer_allocate");
    object_unref = (ObjectUnrefFn)dlsym(RTLD_NEXT, "g_object_unref");
}

static void weak_released(gpointer data, GObject* where_object_was) {
    (void)data;
    (void)where_object_was;
    g_mutex_lock(&lock);
    weak_releases++;
    g_hash_table_remove(live_textures, where_object_was);
    g_mutex_unlock(&lock);
}

CoglTexture* cogl_texture_2d_new_with_size(CoglContext* context, int width, int height) {
    CoglTexture* texture;
    if (!real_texture_new)
        real_texture_new = (TextureNewFn)dlsym(RTLD_NEXT, "cogl_texture_2d_new_with_size");
    texture = real_texture_new ? real_texture_new(context, width, height) : NULL;
    if (texture && running) {
        g_mutex_lock(&lock);
        g_hash_table_add(live_textures, texture);
        if (texture_count < G_N_ELEMENTS(textures)) {
            textures[texture_count] = texture;
            texture_widths[texture_count] = width;
            texture_heights[texture_count] = height;
            texture_count++;
        }
        tracked_textures++;
        g_object_weak_ref(G_OBJECT(texture), weak_released, NULL);
        g_mutex_unlock(&lock);
    }
    return texture;
}

gboolean cogl_framebuffer_allocate(CoglFramebuffer* framebuffer, GError** error) {
    if (!real_framebuffer_allocate)
        real_framebuffer_allocate =
            (FramebufferAllocateFn)dlsym(RTLD_NEXT, "cogl_framebuffer_allocate");
    if (running) {
        allocation_ordinal++;
        if (framebuffer_count < G_N_ELEMENTS(framebuffers)) {
            gboolean found = FALSE;
            for (guint i = 0; i < framebuffer_count; i++)
                if (framebuffers[i] == framebuffer)
                    found = TRUE;
            if (!found) {
                framebuffers[framebuffer_count] = framebuffer;
                framebuffer_ordinals[framebuffer_count] = allocation_ordinal;
                framebuffer_count++;
            }
        }
        if (fail_ordinal && allocation_ordinal == fail_ordinal) {
            g_set_error_literal(error, g_quark_from_static_string("blur-allocation-probe"), 1,
                                "injected TEST-ONLY framebuffer allocation failure");
            return FALSE;
        }
    }
    return real_framebuffer_allocate ? real_framebuffer_allocate(framebuffer, error) : FALSE;
}

static void begin_attempt(guint fail_at) {
    allocation_ordinal = 0;
    framebuffer_count = 0;
    texture_count = 0;
    fail_ordinal = fail_at;
    running = TRUE;
}

static ClutterPaintNode* construct(CoglFramebuffer* fb, int x, int y, guint width, guint height,
                                   float radius, float saturation, float noise,
                                   unsigned char opacity, ClutterBlurCache* cache, guint fail_at) {
    ClutterPaintNode* node;
    begin_attempt(fail_at);
    node = real_new_cached(fb, x, y, width, height, radius, saturation, noise, opacity, cache);
    running = FALSE;
    fail_ordinal = 0;
    return node;
}

static gboolean check(gboolean condition, const char* message) {
    if (!condition && !error_text[0])
        g_strlcpy(error_text, message, sizeof(error_text));
    return condition;
}

static guint live_count(void) {
    guint count;
    g_mutex_lock(&lock);
    count = g_hash_table_size(live_textures);
    g_mutex_unlock(&lock);
    return count;
}

static gboolean same_set(CoglTexture** a, guint na, CoglTexture** b, guint nb) {
    for (guint i = 0; i < na; i++)
        for (guint j = 0; j < nb; j++)
            if (a[i] == b[j])
                return FALSE;
    return na > 0 && nb > 0;
}

static void json_string(FILE* f, const char* s) {
    fputc('"', f);
    for (; *s; s++) {
        if (*s == '"' || *s == '\\')
            fputc('\\', f);
        if ((unsigned char)*s >= 0x20)
            fputc(*s, f);
    }
    fputc('"', f);
}

static void run_checks(CoglFramebuffer* fb, int x, int y, guint width, guint height, float radius,
                       float saturation, float noise, unsigned char opacity) {
    ClutterBlurCache* owner = cache_new();
    ClutterPaintNode *first = NULL, *reuse = NULL, *concurrent = NULL, *replacement = NULL;
    guint baseline_live = 0, retained_live = 0, failed_live = 0;
    guint final_live = 0, total_releases = 0, creations = 0;
    guint build_after_first = 0, build_while_busy = 0, build_after_replace = 0;
    guint released_before = 0;
    guint first_textures_n = 0, concurrent_textures_n = 0;
    CoglTexture *first_textures[64], *concurrent_textures[64];
    guint framebuffer_stages[32], stage_count = 0;
    guint failed_stages = 0;
    int scaled_width = 0, scaled_height = 0;
    guint collision_width = width ^ 1u;
    gboolean collision_checked = FALSE;

    released_before = weak_releases;
    first = construct(fb, x, y, width, height, radius, saturation, noise, opacity, owner, 0);
    if (!check(first != NULL, "initial construction failed"))
        goto out;
    build_after_first = build_count(owner);
    first_textures_n = texture_count;
    scaled_width = texture_widths[0];
    scaled_height = texture_heights[0];
    memcpy(first_textures, textures, first_textures_n * sizeof(textures[0]));
    if (!check(build_after_first == 1, "initial owner build count was not one"))
        goto out;
    node_unref(first);
    first = NULL;

    baseline_live = live_count();
    reuse = construct(fb, x, y, width, height, radius, saturation, noise, opacity, owner, 0);
    if (!check(reuse && build_count(owner) == 1 && texture_count == 0,
               "fixed-geometry reuse allocated targets or changed build count"))
        goto out;

    concurrent = construct(fb, x, y, width, height, radius, saturation, noise, opacity, owner, 0);
    if (!check(concurrent != NULL, "concurrent generation construction failed"))
        goto out;
    build_while_busy = build_count(owner);
    concurrent_textures_n = texture_count;
    memcpy(concurrent_textures, textures, concurrent_textures_n * sizeof(textures[0]));
    if (!check(build_while_busy == 1 && same_set(first_textures, first_textures_n,
                                                 concurrent_textures, concurrent_textures_n),
               "busy owner did not create a disjoint generation while retaining build count"))
        goto out;
    node_unref(concurrent);
    concurrent = NULL;
    node_unref(reuse);
    reuse = NULL;
    retained_live = live_count();
    if (width >= 400 && G_APPROX_VALUE(radius, 24.0f, 0.0001f) &&
        !check(retained_live == 4, "radius 24 / width >= 400 did not retain four target textures"))
        goto out;

    /* Adjacent odd/even source widths collide after factor-two downscaling.
     * They must still replace the full-size capture target. */
    guint before_collision = build_count(owner);
    replacement =
        construct(fb, x, y, collision_width, height, radius, saturation, noise, opacity, owner, 0);
    if (!check(replacement != NULL && texture_count == first_textures_n &&
                   texture_widths[0] == scaled_width && texture_heights[0] == scaled_height &&
                   build_count(owner) == before_collision + 1,
               "source-size collision reused the wrong full-size capture generation"))
        goto out;
    node_unref(replacement);
    replacement = NULL;
    first = construct(fb, x, y, width, height, radius, saturation, noise, opacity, owner, 0);
    if (!check(first != NULL, "could not restore geometry after collision"))
        goto out;
    node_unref(first);
    first = NULL;
    collision_checked = TRUE;
    guint before_replace = build_count(owner);

    /* Observe every distinct framebuffer allocation during a successful replacement. */
    guint changed_width = width + 17;
    replacement =
        construct(fb, x, y, changed_width, height, radius, saturation, noise, opacity, owner, 0);
    if (!check(replacement != NULL, "successful replacement construction failed"))
        goto out;
    stage_count = framebuffer_count;
    if (!check(stage_count == allocation_ordinal, "successful replacement allocation calls did not "
                                                  "map one-to-one to framebuffer objects"))
        goto out;
    if (stage_count > G_N_ELEMENTS(framebuffer_stages))
        stage_count = G_N_ELEMENTS(framebuffer_stages);
    for (guint i = 0; i < stage_count; i++)
        framebuffer_stages[i] = framebuffer_ordinals[i];
    build_after_replace = build_count(owner);
    if (!check(build_after_replace == before_replace + 1,
               "successful replacement did not increment owner count"))
        goto out;
    node_unref(replacement);
    replacement = NULL;

    /* Restore original cached geometry, then test every observed allocation stage. */
    first = construct(fb, x, y, width, height, radius, saturation, noise, opacity, owner, 0);
    if (!check(first != NULL, "could not restore warmed geometry"))
        goto out;
    node_unref(first);
    first = NULL;
    for (guint i = 0; i < stage_count; i++) {
        ClutterPaintNode* failed;
        guint before = build_count(owner);
        guint live_before = live_count();
        failed = construct(fb, x, y, changed_width, height, radius, saturation, noise, opacity,
                           owner, framebuffer_stages[i]);
        if (failed)
            node_unref(failed);
        if (!check(failed == NULL && build_count(owner) == before && live_count() == live_before,
                   "injected replacement failure changed owner or leaked target textures"))
            goto out;
        failed_stages++;
        failed_live = live_count();
        first = construct(fb, x, y, width, height, radius, saturation, noise, opacity, owner, 0);
        if (!check(first != NULL && build_count(owner) == before && texture_count == 0,
                   "old geometry did not reuse after failed replacement"))
            goto out;
        node_unref(first);
        first = NULL;
    }
    check(stage_count > 0, "successful replacement exposed no framebuffer allocation stages");
out:
    if (first)
        node_unref(first);
    if (reuse)
        node_unref(reuse);
    if (concurrent)
        node_unref(concurrent);
    if (replacement)
        node_unref(replacement);
    if (owner)
        object_unref(owner);
    owner = NULL;
    final_live = live_count();
    total_releases = weak_releases - released_before;
    creations = tracked_textures;
    if (!check(final_live == 0 && creations == total_releases, "tracked texture weak releases did "
                                                               "not balance after final owner "
                                                               "release")) { /* The
                                                                                report
                                                                                below
                                                                                records
                                                                                the
                                                                                lifecycle
                                                                                failure.
                                                                              */
    }
    char* temporary = g_strconcat(report_path, ".tmp", NULL);
    FILE* f = fopen(temporary, "w");
    if (!f) {
        fprintf(stderr, "blur-allocation-probe: cannot write report %s\n", report_path);
        g_free(temporary);
        return;
    }
    fprintf(f, "{\"test_only\":true,\"status\":");
    json_string(f, error_text[0] ? "failed" : "passed");
    fprintf(f, ",\"failure\":");
    json_string(f, error_text);
    fprintf(f,
            ",\"input\":{\"src_x\":%d,\"src_y\":%d,\"width\":%u,\"height\":%u,\"radius\":%.9g,"
            "\"saturation\":%.9g,\"noise\":%.9g,\"opacity\":%u}",
            x, y, width, height, radius, saturation, noise, opacity);
    fprintf(
        f,
        ",\"owner_build_count\":{\"initial\":%u,\"busy\":%u,\"after_successful_replacement\":%u}",
        build_after_first, build_while_busy, build_after_replace);
    fprintf(f,
            ",\"scaled_size_collision\":{\"checked\":%s,\"source_widths\":[%u,%u],\"scaled_"
            "texture\":[%d,%d]}",
            collision_checked ? "true" : "false", width, collision_width, scaled_width,
            scaled_height);
    fprintf(f,
            ",\"successful_replacement_allocation_stages\":%u,\"injected_stage_failures\":%u,"
            "\"live_target_textures_after_warm\":%u,\"live_target_textures_after_busy_nodes\":%u,"
            "\"live_target_textures_after_failure\":%u,\"live_target_textures_after_final_"
            "release\":%u,\"weak_texture_releases_total\":%u,\"tracked_texture_creations\":%u,"
            "\"paint_rectangles_added\":0,\"painted_clone_rendering_tested\":false,\"lifecycle_"
            "scope\":\"constructor and generation lifecycle only\"}",
            stage_count, failed_stages, baseline_live, retained_live, failed_live, final_live,
            total_releases, creations);
    fclose(f);
    if (rename(temporary, report_path) != 0) {
        fprintf(stderr, "blur-allocation-probe: cannot publish report %s\n", report_path);
        remove(temporary);
    }
    g_free(temporary);
}

ClutterPaintNode* clutter_blur_node_new_from_framebuffer_cached(
    CoglFramebuffer* framebuffer, int src_x, int src_y, unsigned int width, unsigned int height,
    float radius, float saturation, float noise, unsigned char opacity, ClutterBlurCache* owner) {
    const char* path = getenv("GNOBLIN_TEST_BLUR_STATE_REPORT");
    ClutterPaintNode* node;
    if (!real_new_cached)
        resolve_symbols();
    if (path && *path && !done && !armed && real_new_cached && cache_new && build_count &&
        node_unref && object_unref && real_framebuffer_allocate && real_texture_new) {
        armed = TRUE;
        report_path = path;
        if (!live_textures)
            live_textures = g_hash_table_new(g_direct_hash, g_direct_equal);
        run_checks(framebuffer, src_x, src_y, width, height, radius, saturation, noise, opacity);
        done = TRUE;
    }
    node = real_new_cached ? real_new_cached(framebuffer, src_x, src_y, width, height, radius,
                                             saturation, noise, opacity, owner)
                           : NULL;
    return node;
}
