#define _GNU_SOURCE
#include "gnoblin-vr-common.h"

#include <GL/gl.h>
#include <errno.h>
#include <GL/glx.h>
#include <math.h>
#include <string.h>
#include <X11/Xlib.h>

typedef struct {
    XrSwapchain swapchain;
    XrSwapchainImageOpenGLKHR* images;
    GLuint* framebuffers;
    uint32_t image_count;
    uint32_t width;
    uint32_t height;
} EyeSwapchain;

static gboolean create_swapchains(XrSession session, guint32 view_count,
                                  const XrViewConfigurationView* views, EyeSwapchain* eyes) {
    uint32_t count = 0;
    if (xrEnumerateSwapchainFormats(session, 0, &count, NULL) != XR_SUCCESS || count == 0)
        return FALSE;
    int64_t* formats = g_new0(int64_t, count);
    if (xrEnumerateSwapchainFormats(session, count, &count, formats) != XR_SUCCESS) {
        g_free(formats);
        return FALSE;
    }
    int64_t chosen = 0;
    static const int64_t preferred[] = {0x8058 /* GL_RGBA8 */, 0x8C43 /* GL_SRGB8_ALPHA8 */,
                                        0x8051 /* GL_RGB8 */};
    for (guint p = 0; chosen == 0 && p < G_N_ELEMENTS(preferred); p++)
        for (uint32_t i = 0; i < count; i++)
            if (formats[i] == preferred[p])
                chosen = preferred[p];
    g_free(formats);
    if (chosen == 0)
        return FALSE;
    for (guint32 i = 0; i < view_count; i++) {
        XrSwapchainCreateInfo info = {XR_TYPE_SWAPCHAIN_CREATE_INFO};
        info.usageFlags = XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT;
        info.format = chosen;
        info.width = views[i].recommendedImageRectWidth;
        info.height = views[i].recommendedImageRectHeight;
        info.sampleCount =
            views[i].recommendedSwapchainSampleCount ? views[i].recommendedSwapchainSampleCount : 1;
        info.arraySize = 1;
        info.faceCount = 1;
        info.mipCount = 1;
        eyes[i].width = info.width;
        eyes[i].height = info.height;
        if (xrCreateSwapchain(session, &info, &eyes[i].swapchain) != XR_SUCCESS)
            return FALSE;
        uint32_t image_count = 0;
        if (xrEnumerateSwapchainImages(eyes[i].swapchain, 0, &image_count, NULL) != XR_SUCCESS ||
            image_count == 0)
            return FALSE;
        eyes[i].image_count = image_count;
        eyes[i].images = g_new0(XrSwapchainImageOpenGLKHR, image_count);
        for (uint32_t j = 0; j < image_count; j++)
            eyes[i].images[j].type = XR_TYPE_SWAPCHAIN_IMAGE_OPENGL_KHR;
        if (xrEnumerateSwapchainImages(eyes[i].swapchain, image_count, &image_count,
                                       (XrSwapchainImageBaseHeader*)eyes[i].images) != XR_SUCCESS)
            return FALSE;
        /* The runtime hands over textures; attach each to an application FBO for rendering. */
        eyes[i].framebuffers = g_new0(GLuint, image_count);
        glGenFramebuffers((GLsizei)image_count, eyes[i].framebuffers);
        for (uint32_t j = 0; j < image_count; j++) {
            glBindFramebuffer(GL_FRAMEBUFFER, eyes[i].framebuffers[j]);
            glBindTexture(GL_TEXTURE_2D, eyes[i].images[j].image);
            GLenum probe_error = glGetError();
            GLsizei tex_width = 0, tex_height = 0;
            glGetTexLevelParameteriv(GL_TEXTURE_2D, 0, GL_TEXTURE_WIDTH, &tex_width);
            glGetTexLevelParameteriv(GL_TEXTURE_2D, 0, GL_TEXTURE_HEIGHT, &tex_height);
            GLenum level_error = glGetError();
            (void)probe_error;
            (void)level_error;
            (void)tex_width;
            (void)tex_height;
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D,
                                   eyes[i].images[j].image, 0);
            (void)glGetError();
        }
    }
    return TRUE;
}

/* Compile-once program for the per-eye test pattern. The runtime texture import expects a
 * modern GL context, so rendering uses core objects with a small shader pair. */
static GLuint pattern_program;

static void compile_pattern_program(void) {
    static const char* vertex_source = "#version 450 core\n"
                                       "layout(location=0) in vec2 position;\n"
                                       "out vec2 uv;\n"
                                       "void main() {\n"
                                       "  uv = position * 0.5 + 0.5;\n"
                                       "  gl_Position = vec4(position, 0.0, 1.0);\n"
                                       "}\n";
    static const char* fragment_source =
        "#version 450 core\n"
        "in vec2 uv;\n"
        "uniform float phase;\n"
        "uniform float motion;\n"
        "out vec4 colour;\n"
        "void main() {\n"
        "  float hue = 0.5 + 0.5 * sin(phase * 2.0);\n"
        "  float sweep = step(uv.x, motion);\n"
        "  colour = vec4(0.15 + 0.5 * hue, 0.55, 0.9 - 0.4 * hue, 1.0) * 0.6\n"
        "        + vec4(1.0, 0.6, 0.2, 1.0) * 0.6 * sweep;\n"
        "}\n";
    pattern_program = glCreateProgram();
    GLuint vertex = glCreateShader(GL_VERTEX_SHADER);
    glShaderSource(vertex, 1, &vertex_source, NULL);
    glCompileShader(vertex);
    GLuint fragment = glCreateShader(GL_FRAGMENT_SHADER);
    glShaderSource(fragment, 1, &fragment_source, NULL);
    glCompileShader(fragment);
    glAttachShader(pattern_program, vertex);
    glAttachShader(pattern_program, fragment);
    glLinkProgram(pattern_program);
    glDeleteShader(vertex);
    glDeleteShader(fragment);
}

static void render_eye(const EyeSwapchain* eye, guint32 image_index, guint32 eye_index,
                       double seconds) {
    XrSwapchainImageOpenGLKHR* image = &eye->images[image_index];
    glBindFramebuffer(GL_FRAMEBUFFER, eye->framebuffers[image_index]);
    if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
        /* Late texture completion: relax the mip filter and recheck. */
        glBindTexture(GL_TEXTURE_2D, image->image);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, image->image,
                               0);
        (void)glGetError();
    }
    if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
        static gboolean flagged = FALSE;
        if (!flagged) {
            flagged = TRUE;
            g_printerr(
                "gnoblin-vr: swapchain textures cannot attach in this GL context"
                " (stacks without GL_EXT_memory_object_fd keep the preview dark); status %#x\n",
                (unsigned)glCheckFramebufferStatus(GL_FRAMEBUFFER));
        }
        return;
    }
    glViewport(0, 0, (GLsizei)eye->width, (GLsizei)eye->height);
    if (!pattern_program)
        compile_pattern_program();
    static GLuint pattern_vao = 0, pattern_vbo = 0;
    if (!pattern_vao) {
        static const float quad[] = {-1.0f, -1.0f, 3.0f, -1.0f, -1.0f, 3.0f};
        glGenVertexArrays(1, &pattern_vao);
        glGenBuffers(1, &pattern_vbo);
        glBindVertexArray(pattern_vao);
        glBindBuffer(GL_ARRAY_BUFFER, pattern_vbo);
        glBufferData(GL_ARRAY_BUFFER, sizeof quad, quad, GL_STATIC_DRAW);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, (const void*)0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindVertexArray(0);
    }
    double phase = seconds * 0.4 + (eye_index == 0 ? 0.0 : 1.0 / 3.0);
    float motion = (float)(0.5 + 0.5 * sin(phase * G_PI));
    glUseProgram(pattern_program);
    glUniform1f(glGetUniformLocation(pattern_program, "phase"), (float)phase);
    glUniform1f(glGetUniformLocation(pattern_program, "motion"), motion);
    glBindVertexArray(pattern_vao);
    glDrawArrays(GL_TRIANGLES, 0, 3);
    glBindVertexArray(0);
    glUseProgram(0);
    glFinish();
    static gboolean probed = FALSE;
    if (!probed) {
        probed = TRUE;
        unsigned char pixel[3] = {0, 0, 0};
        glReadPixels((GLint)eye->width / 3, (GLint)eye->height / 3, 1, 1, GL_RGB, GL_UNSIGNED_BYTE,
                     pixel);
        g_printerr("gnoblin-vr: eye %u image %u centre pixel %02x %02x %02x\n", (unsigned)eye_index,
                   (unsigned)image_index, pixel[0], pixel[1], pixel[2]);
    }
}

static void destroy_swapchains(guint32 view_count, EyeSwapchain* eyes) {
    for (guint32 i = 0; i < view_count; i++) {
        if (eyes[i].swapchain != XR_NULL_HANDLE)
            (void)xrDestroySwapchain(eyes[i].swapchain);
        g_free(eyes[i].images);
        if (eyes[i].framebuffers) {
            glDeleteFramebuffers((GLsizei)eyes[i].image_count, eyes[i].framebuffers);
            g_free(eyes[i].framebuffers);
        }
    }
}

typedef GLXContext (*GlXCreateContextAttribsFn)(Display*, GLXFBConfig, GLXContext, Bool,
                                                const int*);

static gboolean setup_gl(Display** display_out, GLXContext* context_out, GLXDrawable* drawable_out,
                         XVisualInfo** visual_out, GLXFBConfig* config_out) {
    Display* display = XOpenDisplay(NULL);
    if (!display) {
        g_printerr("gnoblin-vr: XOpenDisplay failed (DISPLAY=%s XAUTHORITY=%s errno=%s)\n",
                   g_getenv("DISPLAY") ? g_getenv("DISPLAY") : "(unset)",
                   g_getenv("XAUTHORITY") ? g_getenv("XAUTHORITY") : "(unset)", strerror(errno));
        return FALSE;
    }
    int count = 0;
    int glx_attributes[] = {
        GLX_DOUBLEBUFFER, True, GLX_RENDER_TYPE, GLX_RGBA_BIT, GLX_DRAWABLE_TYPE,
        GLX_WINDOW_BIT,   None};
    GLXFBConfig* configs =
        glXChooseFBConfig(display, DefaultScreen(display), glx_attributes, &count);
    if (!configs || count == 0) {
        XCloseDisplay(display);
        g_printerr("gnoblin-vr: no usable GLX framebuffer configuration\n");
        return FALSE;
    }
    GLXFBConfig config = configs[0];
    XFree(configs);
    XVisualInfo* visual = glXGetVisualFromFBConfig(display, config);
    if (!visual) {
        XCloseDisplay(display);
        g_printerr("gnoblin-vr: no GLX visual for the framebuffer configuration\n");
        return FALSE;
    }
    Window root_window = RootWindow(display, visual->screen);
    XSetWindowAttributes attributes = {0};
    attributes.colormap = XCreateColormap(display, root_window, visual->visual, AllocNone);
    attributes.override_redirect = True;
    Window x_window = XCreateWindow(display, root_window, 0, 0, 1, 1, 0, visual->depth, InputOutput,
                                    visual->visual, CWColormap | CWOverrideRedirect, &attributes);
    GlXCreateContextAttribsFn create_attribs;
    if (glXGetProcAddressARB("glXCreateContextAttribsARB"))
        create_attribs =
            (GlXCreateContextAttribsFn)glXGetProcAddressARB("glXCreateContextAttribsARB");
    else
        create_attribs = NULL;
    /* A core 4.5 context keeps the runtime texture import usable on modern GL stacks. */
    GLXContext context = XR_NULL_HANDLE;
    if (create_attribs) {
        int attributes[] = {GLX_CONTEXT_MAJOR_VERSION_ARB,
                            4,
                            GLX_CONTEXT_MINOR_VERSION_ARB,
                            5,
                            GLX_CONTEXT_PROFILE_MASK_ARB,
                            GLX_CONTEXT_CORE_PROFILE_BIT_ARB,
                            None};
        context = create_attribs(display, config, NULL, True, attributes);
    }
    if (!context)
        context = glXCreateNewContext(display, config, GLX_RGBA_TYPE, NULL, True);
    GLXDrawable drawable = glXCreateWindow(display, config, x_window, NULL);
    if (!context || !drawable || !glXMakeCurrent(display, drawable, context)) {
        g_printerr("gnoblin-vr: could not activate the GL context\n");
        XCloseDisplay(display);
        return FALSE;
    }
    *display_out = display;
    *context_out = context;
    *drawable_out = drawable;
    *visual_out = visual;
    *config_out = config;
    return TRUE;
}

int gnoblin_vr_gl_main(void) {
    Display* display = NULL;
    GLXContext glx_context = NULL;
    GLXDrawable drawable = 0;
    XVisualInfo* visual = NULL;
    GLXFBConfig config = NULL;
    if (!setup_gl(&display, &glx_context, &drawable, &visual, &config))
        return 99; /* No GL/X11 stack; main() falls back to the headless session. */

    if (!gnoblin_vr_extension_available(XR_KHR_OPENGL_ENABLE_EXTENSION_NAME))
        return gnoblin_vr_failed("the runtime lacks " XR_KHR_OPENGL_ENABLE_EXTENSION_NAME);
    const char* extensions[] = {XR_KHR_OPENGL_ENABLE_EXTENSION_NAME};
    XrInstanceCreateInfo create_info = {XR_TYPE_INSTANCE_CREATE_INFO};
    g_strlcpy(create_info.applicationInfo.applicationName, "Gnoblin VR",
              sizeof create_info.applicationInfo.applicationName);
    create_info.applicationInfo.applicationVersion = 1;
    g_strlcpy(create_info.applicationInfo.engineName, "Gnoblin",
              sizeof create_info.applicationInfo.engineName);
    create_info.applicationInfo.engineVersion = 1;
    create_info.applicationInfo.apiVersion = XR_CURRENT_API_VERSION;
    create_info.enabledExtensionCount = G_N_ELEMENTS(extensions);
    create_info.enabledExtensionNames = extensions;
    XrInstance instance = XR_NULL_HANDLE;
    XrResult result = xrCreateInstance(&create_info, &instance);
    if (result == XR_ERROR_RUNTIME_UNAVAILABLE || result == XR_ERROR_FORM_FACTOR_UNAVAILABLE)
        return gnoblin_vr_absent("no OpenXR runtime is available");
    if (result != XR_SUCCESS)
        return gnoblin_vr_failed("the OpenXR loader could not create a GL instance");
    XrSystemGetInfo system_info = {XR_TYPE_SYSTEM_GET_INFO};
    system_info.formFactor = XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY;
    XrSystemId system_id = XR_NULL_SYSTEM_ID;
    result = xrGetSystem(instance, &system_info, &system_id);
    if (result == XR_ERROR_FORM_FACTOR_UNAVAILABLE || result == XR_ERROR_SYSTEM_INVALID) {
        xrDestroyInstance(instance);
        return gnoblin_vr_absent("OpenXR is installed, but no headset is available");
    }
    if (result != XR_SUCCESS) {
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the OpenXR runtime could not query a headset");
    }
    PFN_xrVoidFunction requirements_ptr = NULL;
    if (xrGetInstanceProcAddr(instance, "xrGetOpenGLGraphicsRequirementsKHR", &requirements_ptr) !=
            XR_SUCCESS ||
        !requirements_ptr) {
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("xrGetOpenGLGraphicsRequirementsKHR is unavailable");
    }
    XrGraphicsRequirementsOpenGLKHR requirements = {XR_TYPE_GRAPHICS_REQUIREMENTS_OPENGL_KHR};
    if (((PFN_xrGetOpenGLGraphicsRequirementsKHR)requirements_ptr)(instance, system_id,
                                                                   &requirements) != XR_SUCCESS) {
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the runtime rejected the OpenGL requirements query");
    }

    XrGraphicsBindingOpenGLXlibKHR binding = {XR_TYPE_GRAPHICS_BINDING_OPENGL_XLIB_KHR};
    binding.xDisplay = display;
    binding.visualid = (uint32_t)visual->visualid;
    binding.glxFBConfig = config;
    binding.glxContext = glx_context;
    binding.glxDrawable = drawable;
    XrSessionCreateInfo session_info = {XR_TYPE_SESSION_CREATE_INFO};
    session_info.next = &binding;
    session_info.systemId = system_id;
    XrSession session = XR_NULL_HANDLE;
    result = xrCreateSession(instance, &session_info, &session);
    if (result != XR_SUCCESS) {
        xrDestroyInstance(instance);
        return 99; /* GL binding rejected; main() falls back to headless. */
    }

    guint32 view_count = 0;
    result = xrEnumerateViewConfigurationViews(
        instance, system_id, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 0, &view_count, NULL);
    if (result != XR_SUCCESS || view_count < 2 || view_count > 4) {
        (void)xrDestroySession(session);
        xrDestroyInstance(instance);
        return gnoblin_vr_failed(
            "the headset does not report a supported stereo view configuration");
    }
    XrViewConfigurationView* views = g_new0(XrViewConfigurationView, view_count);
    for (guint32 i = 0; i < view_count; i++)
        views[i].type = XR_TYPE_VIEW_CONFIGURATION_VIEW;
    result = xrEnumerateViewConfigurationViews(instance, system_id,
                                               XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO,
                                               view_count, &view_count, views);
    if (result != XR_SUCCESS) {
        g_free(views);
        (void)xrDestroySession(session);
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the OpenXR runtime could not read per-eye modes");
    }
    guint32 widths[4] = {0}, heights[4] = {0};
    for (guint32 i = 0; i < view_count; i++) {
        widths[i] = views[i].recommendedImageRectWidth;
        heights[i] = views[i].recommendedImageRectHeight;
    }

    XrReferenceSpaceCreateInfo space_info = {XR_TYPE_REFERENCE_SPACE_CREATE_INFO};
    space_info.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_LOCAL;
    space_info.poseInReferenceSpace.orientation.w = 1.0f;
    XrSpace space = XR_NULL_HANDLE;
    if (xrCreateReferenceSpace(session, &space_info, &space) != XR_SUCCESS) {
        space_info.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_STAGE;
        if (xrCreateReferenceSpace(session, &space_info, &space) != XR_SUCCESS) {
            g_free(views);
            (void)xrDestroySession(session);
            xrDestroyInstance(instance);
            return gnoblin_vr_failed("the runtime could not create a reference space");
        }
    }

    XrEnvironmentBlendMode blend_mode = XR_ENVIRONMENT_BLEND_MODE_OPAQUE;
    uint32_t blend_count = 0;
    result = xrEnumerateEnvironmentBlendModes(
        instance, system_id, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 0, &blend_count, NULL);
    if (result == XR_SUCCESS && blend_count > 0) {
        XrEnvironmentBlendMode* modes = g_new0(XrEnvironmentBlendMode, blend_count);
        if (xrEnumerateEnvironmentBlendModes(instance, system_id,
                                             XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, blend_count,
                                             &blend_count, modes) == XR_SUCCESS)
            blend_mode = modes[0];
        g_free(modes);
    }

    EyeSwapchain eyes[4] = {0};
    if (!create_swapchains(session, view_count, views, eyes)) {
        destroy_swapchains(view_count, eyes);
        g_free(views);
        (void)xrDestroySession(session);
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the runtime could not create GL swapchains");
    }
    g_free(views);
    gnoblin_vr_write_terminal_status("stopped", "waiting for the OpenXR session to become ready",
                                     view_count, widths, heights);

    gboolean began = FALSE;
    gboolean runtime_lost = FALSE;
    XrCompositionLayerProjectionView layer_views[4];
    XrCompositionLayerProjection projection = {XR_TYPE_COMPOSITION_LAYER_PROJECTION};
    while (!gnoblin_vr_stopping && !runtime_lost) {
        XrEventDataBuffer event = {XR_TYPE_EVENT_DATA_BUFFER};
        result = xrPollEvent(instance, &event);
        if (result == XR_EVENT_UNAVAILABLE) {
            if (!began) {
                g_usleep(10000);
                continue;
            }
        } else if (result != XR_SUCCESS) {
            runtime_lost = TRUE;
            break;
        } else if (event.type == XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED) {
            XrEventDataSessionStateChanged* changed = (XrEventDataSessionStateChanged*)&event;
            if (changed->session != session)
                continue;
            if (changed->state == XR_SESSION_STATE_READY && !began) {
                XrSessionBeginInfo begin_info = {XR_TYPE_SESSION_BEGIN_INFO};
                begin_info.primaryViewConfigurationType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
                if (xrBeginSession(session, &begin_info) != XR_SUCCESS) {
                    runtime_lost = TRUE;
                    break;
                }
                began = TRUE;
                gnoblin_vr_write_terminal_status("running", "GL preview session is active",
                                                 view_count, widths, heights);
                g_message("gnoblin-vr: GL preview session is running (%u views)", view_count);
            } else if (changed->state == XR_SESSION_STATE_STOPPING && began) {
                (void)xrEndSession(session);
                began = FALSE;
                runtime_lost = TRUE;
            } else if (changed->state == XR_SESSION_STATE_EXITING ||
                       changed->state == XR_SESSION_STATE_LOSS_PENDING) {
                runtime_lost = TRUE;
            }
        }
        if (!began)
            continue;
        XrFrameState frame = {XR_TYPE_FRAME_STATE};
        if (xrWaitFrame(session, NULL, &frame) != XR_SUCCESS) {
            runtime_lost = TRUE;
            break;
        }
        XrFrameBeginInfo begin_frame = {XR_TYPE_FRAME_BEGIN_INFO};
        if (xrBeginFrame(session, &begin_frame) != XR_SUCCESS) {
            runtime_lost = TRUE;
            break;
        }
        XrViewLocateInfo locate_info = {XR_TYPE_VIEW_LOCATE_INFO};
        locate_info.viewConfigurationType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
        locate_info.displayTime = frame.predictedDisplayTime;
        locate_info.space = space;
        XrView head_views[4];
        for (guint32 i = 0; i < view_count; i++)
            head_views[i].type = XR_TYPE_VIEW;
        uint32_t located = 0;
        XrViewState view_state = {XR_TYPE_VIEW_STATE};
        if (xrLocateViews(session, &locate_info, &view_state, view_count, &located, head_views) ==
            XR_SUCCESS) {
            for (guint32 i = 0; i < view_count; i++) {
                memset(&layer_views[i], 0, sizeof layer_views[i]);
                layer_views[i].type = XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW;
                layer_views[i].subImage.swapchain = eyes[i].swapchain;
                layer_views[i].subImage.imageRect.offset.x = 0;
                layer_views[i].subImage.imageRect.offset.y = 0;
                layer_views[i].subImage.imageRect.extent.width = (int32_t)eyes[i].width;
                layer_views[i].subImage.imageRect.extent.height = (int32_t)eyes[i].height;
                layer_views[i].subImage.imageArrayIndex = 0;
                layer_views[i].pose = head_views[i].pose;
                layer_views[i].fov = head_views[i].fov;
            }
            projection.viewCount = view_count;
            projection.views = layer_views;
            projection.space = space;
        }
        double seconds = frame.predictedDisplayTime / 1000000000.0;
        for (guint32 i = 0; i < view_count; i++) {
            XrSwapchainImageAcquireInfo acquire_info = {XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO};
            guint32 image_index = 0;
            if (xrAcquireSwapchainImage(eyes[i].swapchain, &acquire_info, &image_index) !=
                XR_SUCCESS)
                continue;
            XrSwapchainImageWaitInfo wait_info = {XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO};
            wait_info.timeout = XR_INFINITE_DURATION;
            if (xrWaitSwapchainImage(eyes[i].swapchain, &wait_info) == XR_SUCCESS)
                render_eye(&eyes[i], image_index, i, seconds);
            XrSwapchainImageReleaseInfo release_info = {XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO};
            (void)xrReleaseSwapchainImage(eyes[i].swapchain, &release_info);
        }
        XrFrameEndInfo end_frame = {XR_TYPE_FRAME_END_INFO};
        end_frame.displayTime = frame.predictedDisplayTime;
        end_frame.environmentBlendMode = blend_mode;
        end_frame.layerCount = 1;
        const XrCompositionLayerBaseHeader* layers[1] = {
            (XrCompositionLayerBaseHeader*)&projection};
        end_frame.layers = layers;
        if (xrEndFrame(session, &end_frame) != XR_SUCCESS)
            runtime_lost = TRUE;
    }

    if (began)
        (void)xrEndSession(session);
    destroy_swapchains(view_count, eyes);
    (void)xrDestroySpace(space);
    (void)xrDestroySession(session);
    (void)xrDestroyInstance(instance);
    gnoblin_vr_write_terminal_status(gnoblin_vr_stopping ? "stopped" : "absent",
                                     gnoblin_vr_stopping ? "stop requested" : "the session ended",
                                     view_count, widths, heights);
    if (gnoblin_vr_stopping) {
        g_message("gnoblin-vr: stopped cleanly");
        return 0;
    }
    return 1;
}
