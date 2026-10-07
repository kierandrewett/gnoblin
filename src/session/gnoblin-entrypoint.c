#include <string.h>

int gnoblin_runtime_main(int argc, char** argv);
int gnoblin_compositor_main(int argc, char** argv);

int main(int argc, char** argv) {
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--gnoblin-runtime-fd") == 0 ||
            strncmp(argv[i], "--gnoblin-runtime-fd=", 21) == 0)
            return gnoblin_compositor_main(argc, argv);
    }

    return gnoblin_runtime_main(argc, argv);
}
