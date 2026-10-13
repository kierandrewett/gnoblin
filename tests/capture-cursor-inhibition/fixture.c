/* SPDX-License-Identifier: GPL-2.0-or-later
 * Real inhibitor mutation, sprite traversal callback, inhibition predicate,
 * and area/monitor enable/disable functions are included verbatim below.
 * Stage, signals and renderer I/O are doubles; no desktop or GPU is started.
 */
#include <glib.h>

typedef struct _Backend MetaBackend;
typedef struct _Stage ClutterStage;
typedef ClutterStage MetaStage;
typedef int MetaStageWatch;
typedef int MetaMonitorManager;
typedef int MetaCursorTracker;

typedef enum {
    META_STREAM_CURSOR_MODE_HIDDEN,
    META_STREAM_CURSOR_MODE_EMBEDDED,
    META_STREAM_CURSOR_MODE_METADATA,
} MetaStreamCursorMode;

typedef struct {
    MetaBackend* backend;
    gboolean visible;
    gboolean hardware;
    gboolean overlay;
    guint refreshes;
} MetaCursorRenderer;

typedef struct {
    MetaCursorRenderer* renderer;
    guint invalidations;
} ClutterSprite;

struct _Stage {
    ClutterSprite sprites[3];
    guint redraws;
};

typedef struct {
    ClutterStage* stage;
    GList* hw_cursor_inhibitors;
    int global_hw_cursor_inhibitors;
} MetaBackendPrivate;

struct _Backend {
    MetaBackendPrivate priv;
    MetaCursorTracker tracker;
    MetaMonitorManager monitor_manager;
};

typedef struct {
    MetaStreamCursorMode mode;
} MetaStream;

typedef struct {
    MetaBackend* backend;
    MetaStream stream;
    gboolean in_stream;
    gboolean hw_cursor_inhibited;
    GList* watches;
    gulong position_invalidated_handler_id;
    gulong cursor_changed_handler_id;
    gulong prepare_frame_handler_id;
    gulong stage_prepare_frame_handler_id;
    guint maybe_record_idle_id;
} MetaStreamSource;

typedef MetaStreamSource MetaStreamSourceArea;
typedef MetaStreamSource MetaStreamSourceMonitor;
typedef MetaStreamSource MetaHwCursorInhibitor;

#define CLUTTER_STAGE(value) (value)
#define CLUTTER_ACTOR(value) (value)
#define META_STAGE(value) (value)
#define META_STREAM_SOURCE_AREA(value) (value)
#define META_STREAM_SOURCE_MONITOR(value) (value)
#define META_HW_CURSOR_INHIBITOR(value) (value)
#define G_CALLBACK(value) NULL

static MetaBackendPrivate* meta_backend_get_instance_private(MetaBackend* backend) {
    return &backend->priv;
}

static gboolean meta_hw_cursor_inhibitor_is_cursor_inhibited(MetaHwCursorInhibitor* inhibitor) {
    /* Geometry is fixed for each case; use the real aggregate predicate. */
    return inhibitor->in_stream;
}

gboolean meta_backend_is_hw_cursors_inhibited(MetaBackend* backend);

static void meta_cursor_renderer_force_update(MetaCursorRenderer* renderer) {
    gboolean inhibited = meta_backend_is_hw_cursors_inhibited(renderer->backend);
    renderer->hardware = renderer->visible && !inhibited;
    renderer->overlay = renderer->visible && inhibited;
    renderer->refreshes++;
}

static void clutter_sprite_invalidate_cursor(ClutterSprite* sprite) {
    sprite->invalidations++;
}

static MetaCursorRenderer* meta_backend_get_cursor_renderer_for_sprite(MetaBackend* backend,
                                                                       ClutterSprite* sprite) {
    return sprite->renderer;
}

static void clutter_stage_foreach_sprite(ClutterStage* stage,
                                         gboolean (*callback)(ClutterStage*, ClutterSprite*,
                                                              gpointer),
                                         gpointer data) {
    for (guint i = 0; i < G_N_ELEMENTS(stage->sprites); i++) {
        if (!callback(stage, &stage->sprites[i], data))
            break;
    }
}

#include "backend.inc"

static MetaBackend* get_backend(MetaStreamSource* source) {
    return source->backend;
}

static ClutterStage* get_stage(MetaStreamSource* source) {
    return source->backend->priv.stage;
}

static MetaMonitorManager* meta_backend_get_monitor_manager(MetaBackend* backend) {
    return &backend->monitor_manager;
}

static MetaCursorTracker* meta_backend_get_cursor_tracker(MetaBackend* backend) {
    return &backend->tracker;
}

static MetaStream* meta_stream_source_get_stream(MetaStreamSource* source) {
    return &source->stream;
}

static MetaStreamCursorMode meta_stream_get_cursor_mode(MetaStream* stream) {
    return stream->mode;
}

static gulong g_signal_connect_after(gpointer object, const char* name, gpointer cb,
                                     gpointer data) {
    return 1;
}

static void g_signal_connect_object(gpointer object, const char* name, gpointer cb, gpointer data,
                                    int flags) {}

static void g_clear_signal_handler(gulong* id, gpointer object) {
    *id = 0;
}

static gboolean mtk_source_remove(guint id) {
    g_assert_not_reached();
}

static void add_view_painted_watches(MetaStreamSource* source) {
    source->watches = g_list_prepend(source->watches, source);
}

static void maybe_reattach_watches(MetaStreamSource* source) {
    add_view_painted_watches(source);
}

static void meta_stage_remove_watch(MetaStage* stage, MetaStageWatch* watch) {}

static void clutter_actor_queue_redraw(ClutterStage* stage) {
    stage->redraws++;
}

/* Separate translation-unit-local names without rewriting their bodies. */
#define inhibit_hw_cursor area_inhibit_hw_cursor
#define uninhibit_hw_cursor area_uninhibit_hw_cursor
#include "area.inc"
#undef inhibit_hw_cursor
#undef uninhibit_hw_cursor
#define inhibit_hw_cursor monitor_inhibit_hw_cursor
#define uninhibit_hw_cursor monitor_uninhibit_hw_cursor
#include "monitor.inc"
#undef inhibit_hw_cursor
#undef uninhibit_hw_cursor

typedef struct {
    ClutterStage stage;
    MetaBackend backend;
    MetaCursorRenderer renderers[2];
} Fixture;

static void setup(Fixture* f) {
    *f = (Fixture){0};
    f->backend.priv.stage = &f->stage;
    for (guint i = 0; i < G_N_ELEMENTS(f->renderers); i++) {
        f->renderers[i] =
            (MetaCursorRenderer){.backend = &f->backend, .visible = TRUE, .hardware = TRUE};
        f->stage.sprites[i].renderer = &f->renderers[i];
    }
    /* The third active sprite deliberately has no renderer. */
}

static MetaStreamSource source_new(Fixture* f, MetaStreamCursorMode mode) {
    return (MetaStreamSource){.backend = &f->backend, .stream.mode = mode, .in_stream = TRUE};
}

static void assert_rendering(Fixture* f, gboolean hardware, gboolean overlay) {
    for (guint i = 0; i < G_N_ELEMENTS(f->renderers); i++) {
        g_assert_cmpint(f->renderers[i].hardware, ==, hardware);
        g_assert_cmpint(f->renderers[i].overlay, ==, overlay);
    }
}

static void finish(Fixture* f, MetaStreamSource* source) {
    g_assert_null(f->backend.priv.hw_cursor_inhibitors);
    g_assert_null(source->watches);
    g_assert_false(source->hw_cursor_inhibited);
    g_assert_cmpuint(source->position_invalidated_handler_id, ==, 0);
    g_assert_cmpuint(source->cursor_changed_handler_id, ==, 0);
    g_assert_cmpuint(source->prepare_frame_handler_id, ==, 0);
    g_assert_cmpuint(source->stage_prepare_frame_handler_id, ==, 0);
}

typedef void (*Lifecycle)(MetaStreamSource*);
typedef struct {
    Lifecycle enable;
    Lifecycle disable;
} StreamKind;

static const StreamKind kinds[] = {
    {meta_stream_source_area_enable, meta_stream_source_area_disable},
    {meta_stream_source_monitor_enable, meta_stream_source_monitor_disable},
};

static void embedded_enable(gconstpointer data) {
    const StreamKind* kind = data;
    Fixture f;
    setup(&f);
    MetaStreamSource source = source_new(&f, META_STREAM_CURSOR_MODE_EMBEDDED);
    kind->enable(&source);
    g_assert_true(source.hw_cursor_inhibited);
    assert_rendering(&f, FALSE, TRUE);
    for (guint i = 0; i < G_N_ELEMENTS(f.stage.sprites); i++)
        g_assert_cmpuint(f.stage.sprites[i].invalidations, >, 0);
    kind->disable(&source);
    finish(&f, &source);
}

static void embedded_disable(gconstpointer data) {
    const StreamKind* kind = data;
    Fixture f;
    setup(&f);
    MetaStreamSource source = source_new(&f, META_STREAM_CURSOR_MODE_EMBEDDED);
    kind->enable(&source);
    /* Establish software rendering independently: isolate the remove seam. */
    for (guint i = 0; i < G_N_ELEMENTS(f.renderers); i++)
        meta_cursor_renderer_force_update(&f.renderers[i]);
    assert_rendering(&f, FALSE, TRUE);
    kind->disable(&source);
    assert_rendering(&f, TRUE, FALSE);
    finish(&f, &source);
}

static void non_embedded(gconstpointer data) {
    const StreamKind* kind = data;
    for (int mode = META_STREAM_CURSOR_MODE_HIDDEN; mode <= META_STREAM_CURSOR_MODE_METADATA;
         mode += 2) {
        Fixture f;
        setup(&f);
        MetaStreamSource source = source_new(&f, mode);
        kind->enable(&source);
        kind->disable(&source);
        assert_rendering(&f, TRUE, FALSE);
        for (guint i = 0; i < G_N_ELEMENTS(f.renderers); i++)
            g_assert_cmpuint(f.renderers[i].refreshes, ==, 0);
        finish(&f, &source);
    }
}

static void overlapping(void) {
    Fixture f;
    setup(&f);
    MetaStreamSource area = source_new(&f, META_STREAM_CURSOR_MODE_EMBEDDED);
    MetaStreamSource monitor = source_new(&f, META_STREAM_CURSOR_MODE_EMBEDDED);
    kinds[0].enable(&area);
    kinds[1].enable(&monitor);
    assert_rendering(&f, FALSE, TRUE);
    kinds[0].disable(&area);
    assert_rendering(&f, FALSE, TRUE);
    kinds[1].disable(&monitor);
    assert_rendering(&f, TRUE, FALSE);
    finish(&f, &area);
    finish(&f, &monitor);
}

static void retain_global_inhibition(void) {
    Fixture f;
    setup(&f);
    f.backend.priv.global_hw_cursor_inhibitors = 1;
    MetaStreamSource source = source_new(&f, META_STREAM_CURSOR_MODE_EMBEDDED);
    kinds[0].enable(&source);
    kinds[0].disable(&source);
    assert_rendering(&f, FALSE, TRUE);
    finish(&f, &source);
}

static void keep_invisible_cursor_hidden(void) {
    Fixture f;
    setup(&f);
    for (guint i = 0; i < G_N_ELEMENTS(f.renderers); i++) {
        f.renderers[i].visible = FALSE;
        f.renderers[i].hardware = FALSE;
    }
    MetaStreamSource source = source_new(&f, META_STREAM_CURSOR_MODE_EMBEDDED);
    kinds[0].enable(&source);
    assert_rendering(&f, FALSE, FALSE);
    kinds[0].disable(&source);
    assert_rendering(&f, FALSE, FALSE);
    finish(&f, &source);
}

static void cursor_outside_stream(void) {
    Fixture f;
    setup(&f);
    MetaStreamSource source = source_new(&f, META_STREAM_CURSOR_MODE_EMBEDDED);
    source.in_stream = FALSE;
    kinds[0].enable(&source);
    assert_rendering(&f, TRUE, FALSE);
    kinds[0].disable(&source);
    assert_rendering(&f, TRUE, FALSE);
    finish(&f, &source);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_data_func("/area/embedded-enable", &kinds[0], embedded_enable);
    g_test_add_data_func("/area/embedded-disable", &kinds[0], embedded_disable);
    g_test_add_data_func("/area/hidden-and-metadata", &kinds[0], non_embedded);
    g_test_add_data_func("/monitor/embedded-enable", &kinds[1], embedded_enable);
    g_test_add_data_func("/monitor/embedded-disable", &kinds[1], embedded_disable);
    g_test_add_data_func("/monitor/hidden-and-metadata", &kinds[1], non_embedded);
    g_test_add_func("/inhibitors/overlapping", overlapping);
    g_test_add_func("/inhibitors/retain-global", retain_global_inhibition);
    g_test_add_func("/inhibitors/invisible-cursor", keep_invisible_cursor_hidden);
    g_test_add_func("/inhibitors/cursor-outside", cursor_outside_stream);
    return g_test_run();
}
