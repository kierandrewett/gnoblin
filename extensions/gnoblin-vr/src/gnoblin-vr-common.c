#define _GNU_SOURCE
#include "gnoblin-vr-common.h"

#include <string.h>

volatile gboolean gnoblin_vr_stopping = 0;

void gnoblin_vr_handle_stop(int signal_number) {
    (void)signal_number;
    gnoblin_vr_stopping = 1;
}

void gnoblin_vr_write_terminal_status(const char* state, const char* diagnostic, guint count,
                                      const guint32* widths, const guint32* heights) {
    g_autoptr(GError) error = NULL;
    if (!gnoblin_vr_status_write(state, diagnostic, count, widths, heights, &error))
        g_printerr("gnoblin-vr: cannot write status: %s\n", error->message);
}

void gnoblin_vr_select_monado_service(void) {
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

gboolean gnoblin_vr_extension_available(const char* extension) {
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

int gnoblin_vr_absent(const char* diagnostic) {
    g_message("gnoblin-vr: %s", diagnostic);
    gnoblin_vr_write_terminal_status("absent", diagnostic, 0, NULL, NULL);
    return 0;
}

int gnoblin_vr_failed(const char* diagnostic) {
    g_printerr("gnoblin-vr: %s\n", diagnostic);
    gnoblin_vr_write_terminal_status("error", diagnostic, 0, NULL, NULL);
    return 1;
}
