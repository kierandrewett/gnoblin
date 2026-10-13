/* Real Cogl/GLES probe for blur-kernel-capacity.py. Include the selected
 * ClutterBlur implementation verbatim; no shader or uniform-call stubs. */
#include <GLES2/gl2.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <stdio.h>
#include "cogl/cogl.h"
#include BLUR_SOURCE

typedef struct {
    CoglDisplayEGL parent_instance;
} AuditDisplay;
typedef struct {
    CoglDisplayEGLClass parent_class;
} AuditDisplayClass;
GType audit_display_get_type(void);
G_DEFINE_TYPE(AuditDisplay, audit_display, COGL_TYPE_DISPLAY_EGL)

static int audit_config_attributes(CoglDisplayEGL* display, EGLint* attributes) {
    return 0;
}

static void audit_display_class_init(AuditDisplayClass* klass) {
    COGL_DISPLAY_EGL_CLASS(klass)->add_config_attributes = audit_config_attributes;
}

static void audit_display_init(AuditDisplay* display) {}

static void check_link(GLuint program) {
    GLint status;
    char log[4096];

    glGetProgramiv(program, GL_LINK_STATUS, &status);
    if (status != GL_TRUE) {
        glGetProgramInfoLog(program, sizeof(log), NULL, log);
        g_error("link: %s", log);
    }
}

static void capture_program(const char* directory) {
    /* A uniform-free vertex stage makes reflection specific to the exact
     * generated fragment shader. Never bind this inspection-only program. */
    const char* vertex_source = "#version 100\n"
                                "attribute vec4 position;\n"
                                "varying vec4 _cogl_color;\n"
                                "varying vec4 _cogl_tex_coord[1];\n"
                                "void main() { gl_Position = position; _cogl_color = vec4(1.0);\n"
                                "_cogl_tex_coord[0] = position; }\n";
    GLint program, n, status;
    GLuint shaders[8], fragment = 0;
    GLsizei count;
    int rows = 0;

    glGetIntegerv(GL_CURRENT_PROGRAM, &program);
    check_link(program);
    glGetAttachedShaders(program, G_N_ELEMENTS(shaders), &count, shaders);
    for (int i = 0; i < count; i++) {
        GLint length, type;
        glGetShaderiv(shaders[i], GL_SHADER_TYPE, &type);
        glGetShaderiv(shaders[i], GL_SHADER_SOURCE_LENGTH, &length);
        g_autofree char* source = g_malloc(length);
        g_autofree char* path = g_strdup_printf("%s/%s.glsl", directory,
                                                type == GL_FRAGMENT_SHADER ? "fragment" : "vertex");
        glGetShaderSource(shaders[i], length, NULL, source);
        g_assert(g_file_set_contents(path, source, -1, NULL));
        if (type == GL_FRAGMENT_SHADER)
            fragment = shaders[i];
    }
    g_assert_cmpuint(fragment, !=, 0);

    GLuint vertex = glCreateShader(GL_VERTEX_SHADER);
    glShaderSource(vertex, 1, &vertex_source, NULL);
    glCompileShader(vertex);
    glGetShaderiv(vertex, GL_COMPILE_STATUS, &status);
    g_assert_cmpint(status, ==, GL_TRUE);
    GLuint inspection = glCreateProgram();
    glAttachShader(inspection, vertex);
    glAttachShader(inspection, fragment);
    glLinkProgram(inspection);
    check_link(inspection);
    glGetProgramiv(inspection, GL_ACTIVE_UNIFORMS, &n);
    for (int i = 0; i < n; i++) {
        char name[256];
        GLint size;
        GLenum type;
        glGetActiveUniform(inspection, i, sizeof(name), NULL, &size, &type, name);
        /* One entire vec4 slot per scalar/vector/sampler; no packing assumed.
         * Fail closed for matrices or new types, rather than undercounting. */
        g_assert(type == GL_FLOAT || type == GL_FLOAT_VEC2 || type == GL_FLOAT_VEC3 ||
                 type == GL_FLOAT_VEC4 || type == GL_SAMPLER_2D);
        rows += size;
        printf("FRAGMENT_UNIFORM %s %d 0x%x\n", name, size, type);
    }
    printf("FRAGMENT_ROWS %d\n", rows);
    g_assert_cmpint(rows, <=, 16);
    glDeleteProgram(inspection);
    glDeleteShader(vertex);
}

static const struct {
    int width, height;
    float radius;
} cases[] = {
    {1280, 82, 24.000011f}, {67, 31, 2.f},        {1280, 139, 24.000017f}, {65, 33, 26.666666f},
    {67, 31, 0.5f},         {65, 33, 26.666668f}, {67, 31, 24.f},          {1, 19, 32.f},
    {67, 31, 1.9999999f},   {19, 1, 2.f},         {513, 257, 48.f},
};

int main(int argc, char** argv) {
    g_assert_cmpint(argc, ==, 2);
    PFNEGLGETPLATFORMDISPLAYEXTPROC platform = (void*)eglGetProcAddress("eglGetPlatformDisplayEXT");
    g_assert(platform);
    EGLDisplay edpy = platform(EGL_PLATFORM_SURFACELESS_MESA, EGL_DEFAULT_DISPLAY, NULL);
    g_autoptr(GError) error = NULL;
    CoglRendererEGL* renderer = cogl_renderer_egl_new();
    cogl_renderer_egl_set_edisplay(renderer, edpy);
    if (!cogl_renderer_connect(COGL_RENDERER(renderer), &error))
        g_error("renderer: %s", error->message);
    CoglDisplayEGL* display = g_object_new(audit_display_get_type(), "renderer", renderer, NULL);
    if (!cogl_display_setup(COGL_DISPLAY(display), &error))
        g_error("display: %s", error->message);
    g_assert(cogl_display_egl_make_current(display, EGL_NO_SURFACE, EGL_NO_SURFACE,
                                           cogl_display_egl_get_egl_context(display)));
    CoglContext* ctx = cogl_context_egl_new(COGL_DISPLAY(display), &error);
    if (!ctx)
        g_error("context: %s", error->message);
    GLint vectors;
    glGetIntegerv(GL_MAX_FRAGMENT_UNIFORM_VECTORS, &vectors);
    printf("GL_VERSION %s\nGLSL_VERSION %s\nRENDERER %s\nDRIVER_LIMIT %d\n",
           glGetString(GL_VERSION), glGetString(GL_SHADING_LANGUAGE_VERSION),
           glGetString(GL_RENDERER), vectors);

    ClutterBlur* blurs[G_N_ELEMENTS(cases)];
    for (int k = 0; k < G_N_ELEMENTS(cases); k++) {
        int width = cases[k].width, height = cases[k].height;
        g_autofree guint8* pixels = g_malloc(width * height * 4);
        for (int y = 0; y < height; y++)
            for (int x = 0; x < width; x++) {
                int i = (y * width + x) * 4;
                int alpha = (x * 13 + y * 29) % 256;
                if (x == 0 || y == 0 || x == width - 1 || y == height - 1)
                    alpha = 255;
                pixels[i] = ((x / 7 + y / 5) % 2) * alpha;
                pixels[i + 1] = (x * 17 % 256) * alpha / 255;
                pixels[i + 2] = (y * 31 % 256) * alpha / 255;
                pixels[i + 3] = alpha;
            }
        CoglTexture* source = cogl_texture_2d_new_from_data(
            ctx, width, height, COGL_PIXEL_FORMAT_RGBA_8888_PRE, width * 4, pixels, &error);
        if (!source)
            g_error("texture: %s", error->message);
        blurs[k] = clutter_blur_new(source, cases[k].radius);
        g_assert(blurs[k]);
        g_object_unref(source);
    }

    /* Retain all instances, alternating full/short array uploads and fallback
     * across Cogl pipeline copies sharing one generated program. */
    for (int round = 0; round < 3; round++)
        for (int j = 0; j < G_N_ELEMENTS(cases); j++) {
            int k = round == 1 ? G_N_ELEMENTS(cases) - 1 - j : j;
            ClutterBlur* blur = blurs[k];
            CoglFramebuffer* fb = blur->pass[HORIZONTAL].framebuffer;
            int width = cogl_framebuffer_get_width(fb);
            int height = cogl_framebuffer_get_height(fb);
            g_autofree guint8* actual = g_malloc(width * height * 4);
            g_autofree char* path =
                g_strdup_printf("%s/case-%02d-round-%d.rgba", argv[1], k, round);
            clutter_blur_apply(blur);
            cogl_framebuffer_finish(fb);
            if (round == 0 && j == 0)
                capture_program(argv[1]);
            g_assert(cogl_framebuffer_read_pixels(fb, 0, 0, width, height,
                                                  COGL_PIXEL_FORMAT_RGBA_8888_PRE, actual));
            g_assert(g_file_set_contents(path, (char*)actual, width * height * 4, NULL));
            GLint program;
            glGetIntegerv(GL_CURRENT_PROGRAM, &program);
            printf("CASE %d %d %d %d %.9g %d\n", k, round, width, height, cases[k].radius, program);
            g_assert_cmpint(glGetError(), ==, GL_NO_ERROR);
        }
    for (int k = 0; k < G_N_ELEMENTS(cases); k++)
        clutter_blur_free(blurs[k]);
    g_object_unref(ctx);
    g_object_unref(display);
    g_object_unref(renderer);
    return 0;
}
