#define _GNU_SOURCE
#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>

#include <glib.h>
#include <signal.h>
#include <sys/stat.h>
#include <unistd.h>

#include "gnoblin-vr-status.h"

static volatile sig_atomic_t stopping;

static void handle_stop(int signal_number) {
    (void)signal_number;
    stopping = 1;
}

static void write_terminal_status(const char* state, const char* diagnostic, guint count,
                                  const guint32* widths, const guint32* heights) {
    g_autoptr(GError) error = NULL;
    if (!gnoblin_vr_status_write(state, diagnostic, count, widths, heights, &error))
        g_printerr("gnoblin-vr: cannot write status: %s\n", error->message);
}

static void select_monado_service(void) {
    if (g_getenv("XR_RUNTIME_JSON") && *g_getenv("XR_RUNTIME_JSON"))
        return;
    const char* runtime_dir = g_get_user_runtime_dir();
    g_autofree char* ipc = g_build_filename(runtime_dir, "monado_comp_ipc", NULL);
    struct stat stat_buffer;
    if (stat(ipc, &stat_buffer) != 0 || !S_ISSOCK(stat_buffer.st_mode))
        return;

    const char* manifests[] = {
        "/usr/share/openxr/1/openxr_monado.json", "/usr/local/share/openxr/1/openxr_monado.json",
        "/usr/share/openxr/1/monado.json", "/usr/local/share/openxr/1/monado.json", NULL};
    for (guint i = 0; manifests[i]; i++) {
        if (!g_file_test(manifests[i], G_FILE_TEST_IS_REGULAR))
            continue;
        if (g_setenv("XR_RUNTIME_JSON", manifests[i], FALSE))
            g_message("gnoblin-vr: using Monado service at %s with runtime %s", ipc, manifests[i]);
        return;
    }
    g_message("gnoblin-vr: Monado service socket is present at %s; using loader default runtime",
              ipc);
}

static gboolean extension_available(const char* extension) {
    uint32_t count = 0;
    if (xrEnumerateInstanceExtensionProperties(NULL, 0, &count, NULL) != XR_SUCCESS)
        return FALSE;
    XrExtensionProperties* properties = g_new0(XrExtensionProperties, count);
    for (uint32_t i = 0; i < count; i++)
        properties[i].type = XR_TYPE_EXTENSION_PROPERTIES;
    XrResult result = xrEnumerateInstanceExtensionProperties(NULL, count, &count, properties);
    gboolean found = FALSE;
    if (result == XR_SUCCESS)
        for (uint32_t i = 0; i < count; i++)
            found |= g_str_equal(properties[i].extensionName, extension);
    g_free(properties);
    return found;
}

static int absent(const char* diagnostic) {
    g_message("gnoblin-vr: %s", diagnostic);
    write_terminal_status("absent", diagnostic, 0, NULL, NULL);
    return 0;
}

static int failed(const char* diagnostic) {
    g_printerr("gnoblin-vr: %s\n", diagnostic);
    write_terminal_status("error", diagnostic, 0, NULL, NULL);
    return 1;
}

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
    if (argc != 1) {
        g_printerr("usage: gnoblin-vr [--status]\n");
        return 2;
    }
    signal(SIGTERM, handle_stop);
    signal(SIGINT, handle_stop);
    /* Keep phase-2 preview behaviour extension-owned. Monado opens its XCB
     * preview window and does not stream this session to a headset. */
    if (!g_getenv("XRT_COMPOSITOR_FORCE_XCB"))
        g_setenv("XRT_COMPOSITOR_FORCE_XCB", "1", FALSE);
    select_monado_service();

    XrInstance instance = XR_NULL_HANDLE;
    XrInstanceCreateInfo create_info = {XR_TYPE_INSTANCE_CREATE_INFO};
    g_strlcpy(create_info.applicationInfo.applicationName, "Gnoblin VR",
              sizeof create_info.applicationInfo.applicationName);
    create_info.applicationInfo.applicationVersion = 1;
    g_strlcpy(create_info.applicationInfo.engineName, "Gnoblin",
              sizeof create_info.applicationInfo.engineName);
    create_info.applicationInfo.engineVersion = 1;
    create_info.applicationInfo.apiVersion = XR_CURRENT_API_VERSION;
    XrResult result = xrCreateInstance(&create_info, &instance);
    if (result == XR_ERROR_RUNTIME_UNAVAILABLE || result == XR_ERROR_FORM_FACTOR_UNAVAILABLE)
        return absent("no OpenXR runtime is available");
    if (result != XR_SUCCESS)
        return failed("the OpenXR loader could not create an instance");

    XrSystemGetInfo system_info = {XR_TYPE_SYSTEM_GET_INFO};
    system_info.formFactor = XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY;
    XrSystemId system_id = XR_NULL_SYSTEM_ID;
    result = xrGetSystem(instance, &system_info, &system_id);
    if (result == XR_ERROR_FORM_FACTOR_UNAVAILABLE || result == XR_ERROR_SYSTEM_INVALID) {
        xrDestroyInstance(instance);
        return absent("OpenXR is installed, but no headset is available");
    }
    if (result != XR_SUCCESS) {
        xrDestroyInstance(instance);
        return failed("the OpenXR runtime could not query a headset");
    }

    if (!extension_available(XR_MND_HEADLESS_EXTENSION_NAME)) {
        xrDestroyInstance(instance);
        return failed(
            "the runtime lacks XR_MND_headless, required for a session that renders no scene");
    }
    xrDestroyInstance(instance);
    const char* extensions[] = {XR_MND_HEADLESS_EXTENSION_NAME};
    create_info.enabledExtensionCount = G_N_ELEMENTS(extensions);
    create_info.enabledExtensionNames = extensions;
    instance = XR_NULL_HANDLE;
    result = xrCreateInstance(&create_info, &instance);
    if (result != XR_SUCCESS)
        return failed("the OpenXR runtime rejected XR_MND_headless");
    result = xrGetSystem(instance, &system_info, &system_id);
    if (result != XR_SUCCESS) {
        xrDestroyInstance(instance);
        return absent("the headset became unavailable while opening the session");
    }

    uint32_t view_count = 0;
    result = xrEnumerateViewConfigurationViews(
        instance, system_id, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 0, &view_count, NULL);
    if (result != XR_SUCCESS || view_count < 2 || view_count > 4) {
        xrDestroyInstance(instance);
        return failed("the headset does not report a supported stereo view configuration");
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
        return failed("the OpenXR runtime could not read per-eye modes");
    }

    XrSessionCreateInfo session_info = {XR_TYPE_SESSION_CREATE_INFO};
    session_info.systemId = system_id;
    XrSession session = XR_NULL_HANDLE;
    result = xrCreateSession(instance, &session_info, &session);
    if (result != XR_SUCCESS) {
        xrDestroyInstance(instance);
        g_free(views);
        return failed("the OpenXR runtime could not open a headless headset session");
    }

    guint32 widths[4] = {0}, heights[4] = {0};
    for (uint32_t i = 0; i < view_count; i++) {
        widths[i] = views[i].recommendedImageRectWidth;
        heights[i] = views[i].recommendedImageRectHeight;
    }
    g_free(views);
    write_terminal_status("stopped", "waiting for the OpenXR session to become ready", view_count,
                          widths, heights);

    gboolean began = FALSE;
    gboolean runtime_lost = FALSE;
    while (!stopping && !runtime_lost) {
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
                write_terminal_status("running", "headless OpenXR session is active", view_count,
                                      widths, heights);
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
    if (stopping) {
        write_terminal_status("stopped", "stopped by the session", view_count, widths, heights);
        g_message("gnoblin-vr: stopped cleanly");
        return 0;
    }
    return absent("the OpenXR headset session ended or the headset disconnected");
}
