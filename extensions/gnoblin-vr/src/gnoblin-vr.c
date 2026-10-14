#define _GNU_SOURCE
#include "gnoblin-vr-common.h"
#include "gnoblin-vr-session.h"

int main(int argc, char** argv) {
    if (argc == 2 && g_str_equal(argv[1], "--status")) {
        g_autofree char* path = gnoblin_vr_status_path();
        g_autofree char* contents = NULL;
        gsize length = 0;
        if (!g_file_get_contents(path, &contents, &length, NULL)) {
            g_print("[vr]\nstate=absent\ndiagnostic=client has not run\n");
            return 0;
        }
        if (length)
            g_print("%s", contents);
        return 0;
    }
    if (argc == 2 && g_str_equal(argv[1], "--devices"))
        return gnoblin_vr_devices_main();
    if (argc == 2 && g_str_equal(argv[1], "--watch"))
        return gnoblin_vr_watch_main();
    if ((argc == 2 || argc == 3) && g_str_equal(argv[1], "--connect"))
        return gnoblin_vr_connect_main(argc == 3 ? argv[2] : NULL);
    if (argc == 3 && g_str_equal(argv[1], "--runtime"))
        return gnoblin_vr_runtime_main(argv[2]);
    const char* backend = NULL;
    if (argc == 3 && g_str_equal(argv[1], "--backend"))
        backend = argv[2];
    else if (argc != 1) {
        g_printerr("usage: gnoblin-vr [--status | --devices | --watch | --connect [ID] |\n"
                   "                  --runtime monado|wivrn | --backend monado|wivrn]\n");
        return 2;
    }
    if (backend && !gnoblin_vr_select_backend(backend))
        return gnoblin_vr_absent("the requested OpenXR runtime is not installed");
    signal(SIGTERM, gnoblin_vr_handle_stop);
    signal(SIGINT, gnoblin_vr_handle_stop);
    /* Keep phase-2 preview behaviour extension-owned. Monado opens its XCB
     * preview window and does not stream this session to a headset. */
    if (!backend && !g_getenv("XRT_COMPOSITOR_FORCE_XCB"))
        g_setenv("XRT_COMPOSITOR_FORCE_XCB", "1", FALSE);
    if (!backend)
        gnoblin_vr_select_monado_service();

    /* A bare instance lets an absent runtime exit cleanly before any extension
     * checks try to query properties from a loader that has no runtime. */
    XrInstanceCreateInfo probe_info = {XR_TYPE_INSTANCE_CREATE_INFO};
    g_strlcpy(probe_info.applicationInfo.applicationName, "Gnoblin VR",
              sizeof probe_info.applicationInfo.applicationName);
    probe_info.applicationInfo.applicationVersion = 1;
    g_strlcpy(probe_info.applicationInfo.engineName, "Gnoblin",
              sizeof probe_info.applicationInfo.engineName);
    probe_info.applicationInfo.engineVersion = 1;
    probe_info.applicationInfo.apiVersion = XR_CURRENT_API_VERSION;
    XrInstance probe_instance = XR_NULL_HANDLE;
    XrResult probe = xrCreateInstance(&probe_info, &probe_instance);
    if (probe == XR_ERROR_RUNTIME_UNAVAILABLE || probe == XR_ERROR_FORM_FACTOR_UNAVAILABLE) {
        if (probe_instance != XR_NULL_HANDLE)
            (void)xrDestroyInstance(probe_instance);
        return gnoblin_vr_absent("no OpenXR runtime is available");
    }
    if (probe == XR_SUCCESS) {
        (void)xrDestroyInstance(probe_instance);
    } else {
        return gnoblin_vr_failed("the OpenXR loader could not create an instance");
    }

    /* The GL path renders the animated per-eye test pattern through real
     * swapchains so the preview window shows both eyes. Fall back to the
     * headless session when the GL/X11 stack or the runtime GL extension is
     * unavailable. */
    if (gnoblin_vr_extension_available(XR_KHR_OPENGL_ENABLE_EXTENSION_NAME)) {
        int gl_result = gnoblin_vr_gl_main();
        if (gl_result != 99)
            return gl_result;
        g_message(
            "gnoblin-vr: GL presentation is unavailable; falling back to the headless session");
    }

    if (!gnoblin_vr_extension_available(XR_MND_HEADLESS_EXTENSION_NAME))
        return gnoblin_vr_failed(
            "the runtime lacks XR_MND_headless, required for a session that renders no scene");
    const char* extensions[] = {XR_MND_HEADLESS_EXTENSION_NAME};
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
        return gnoblin_vr_failed("the OpenXR loader could not create an instance");

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

    uint32_t view_count = 0;
    result = xrEnumerateViewConfigurationViews(
        instance, system_id, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 0, &view_count, NULL);
    if (result != XR_SUCCESS || view_count < 2 || view_count > 4) {
        xrDestroyInstance(instance);
        return gnoblin_vr_failed(
            "the headset does not report a supported stereo view configuration");
    }
    XrViewConfigurationView* views = g_new0(XrViewConfigurationView, view_count);
    for (uint32_t i = 0; i < view_count; i++)
        views[i].type = XR_TYPE_VIEW_CONFIGURATION_VIEW;
    result = xrEnumerateViewConfigurationViews(instance, system_id,
                                               XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO,
                                               view_count, &view_count, views);
    if (result != XR_SUCCESS) {
        g_free(views);
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the OpenXR runtime could not read per-eye modes");
    }

    XrSessionCreateInfo session_info = {XR_TYPE_SESSION_CREATE_INFO};
    session_info.systemId = system_id;
    XrSession session = XR_NULL_HANDLE;
    result = xrCreateSession(instance, &session_info, &session);
    if (result != XR_SUCCESS) {
        xrDestroyInstance(instance);
        g_free(views);
        return gnoblin_vr_failed("the OpenXR runtime could not open a headless headset session");
    }

    guint32 widths[4] = {0}, heights[4] = {0};
    for (uint32_t i = 0; i < view_count; i++) {
        widths[i] = views[i].recommendedImageRectWidth;
        heights[i] = views[i].recommendedImageRectHeight;
    }
    g_free(views);
    gnoblin_vr_write_terminal_status("stopped", "waiting for the OpenXR session to become ready",
                                     view_count, widths, heights);

    gboolean began = FALSE;
    gboolean runtime_lost = FALSE;
    while (!gnoblin_vr_stopping && !runtime_lost) {
        XrEventDataBuffer event = {XR_TYPE_EVENT_DATA_BUFFER};
        result = xrPollEvent(instance, &event);
        if (result == XR_EVENT_UNAVAILABLE) {
            g_usleep(10000);
            continue;
        }
        if (result != XR_SUCCESS) {
            runtime_lost = TRUE;
            break;
        }
        if (event.type == XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED) {
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
                gnoblin_vr_write_terminal_status("running", "headless OpenXR session is active",
                                                 view_count, widths, heights);
                g_message("gnoblin-vr: OpenXR headset session is running (%u views)", view_count);
            } else if (changed->state == XR_SESSION_STATE_STOPPING && began) {
                (void)xrEndSession(session);
                began = FALSE;
                runtime_lost = TRUE;
            } else if (changed->state == XR_SESSION_STATE_EXITING ||
                       changed->state == XR_SESSION_STATE_LOSS_PENDING) {
                runtime_lost = TRUE;
            }
        }
    }

    if (began)
        (void)xrEndSession(session);
    (void)xrDestroySession(session);
    (void)xrDestroyInstance(instance);
    if (gnoblin_vr_stopping) {
        gnoblin_vr_write_terminal_status("stopped", "stopped by the session", view_count, widths,
                                         heights);
        g_message("gnoblin-vr: stopped cleanly");
        return 0;
    }
    return gnoblin_vr_absent("the OpenXR headset session ended or the headset disconnected");
}
