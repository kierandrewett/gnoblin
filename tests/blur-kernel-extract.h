/* Minimal Cogl uniform sink for blur-kernel-gpu.py. The shader strings,
 * downscale calculation and uniform setter are compiled from the real source.
 * No GL, compositor, framebuffer allocation or session calls happen here. */
#include <glib.h>
#include <float.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    int width, height;
} CoglTexture;
typedef void CoglFramebuffer;
typedef void CoglPipeline;
typedef struct _ClutterBlur ClutterBlur;

static char* uniform_names[32];
static int n_uniforms;

static int cogl_texture_get_width(CoglTexture* texture) {
    return texture->width;
}

static int cogl_texture_get_height(CoglTexture* texture) {
    return texture->height;
}

static int cogl_pipeline_get_uniform_location(CoglPipeline* pipeline, const char* name) {
    for (int i = 0; i < n_uniforms; i++)
        if (g_str_equal(uniform_names[i], name))
            return i;
    g_assert(n_uniforms < (int)G_N_ELEMENTS(uniform_names));
    uniform_names[n_uniforms] = g_strdup(name);
    return n_uniforms++;
}

static void cogl_pipeline_set_uniform_float(CoglPipeline* pipeline, int location, int components,
                                            int count, const float* value) {
    printf("{\"name\":\"%s\",\"components\":%d,\"count\":%d,\"value\":[", uniform_names[location],
           components, count);
    for (int i = 0; i < components * count; i++)
        printf("%s%.9g", i ? "," : "", value[i]);
    puts("]}");
}

static void cogl_pipeline_set_uniform_1f(CoglPipeline* pipeline, int location, float value) {
    cogl_pipeline_set_uniform_float(pipeline, location, 1, 1, &value);
}

static void G_GNUC_UNUSED cogl_pipeline_set_uniform_1i(CoglPipeline* pipeline, int location,
                                                       int value) {
    printf("{\"name\":\"%s\",\"integer\":true,\"value\":[%d]}\n", uniform_names[location], value);
}
