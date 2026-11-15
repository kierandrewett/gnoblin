/*
 * An LD_PRELOAD shim for a root file system that has no /proc (tests/parity/README.md, "Run the
 * Mutter reference").
 *
 * The legacy Gnoblin guardian starts its supervisor and its Lua worker by running itself again, and
 * finds itself by reading the link /proc/self/exe (src/session/gnoblin-runtime*.c). A chroot on an
 * Agent37 instance cannot mount /proc, so that read fails and the guardian refuses to start. This
 * library answers that one link with the path the process was started with (the kernel's AT_EXECFN,
 * made absolute) and passes every other readlink through, so the compositor, the Lua runtime and
 * the control socket that run behind it are the real ones and nothing is substituted for them.
 *
 * Build: cc -shared -fPIC -O2 -o parity-proc-shim.so parity-proc-shim.c -ldl
 * Use:   GNOBLIN_PARITY_PROC_SHIM=/path/to/parity-proc-shim.so scripts/parity-mutter-gnoblin
 * RUNTIME_DIR DISPLAY
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <sys/auxv.h>
#include <unistd.h>

static int is_self_exe(const char* path) {
    return path != NULL && strcmp(path, "/proc/self/exe") == 0;
}

/* The absolute path the process was executed by, or NULL when the kernel did not say or it cannot
 * be resolved. */
static const char* self_exe(char* resolved) {
    const char* execfn = (const char*)getauxval(AT_EXECFN);

    if (execfn == NULL)
        return NULL;
    return realpath(execfn, resolved);
}

static ssize_t answer(const char* exe, char* buf, size_t size) {
    size_t length = strlen(exe);

    /* readlink(2) truncates without a terminator and does not report an error for a short buffer.
     */
    if (length > size)
        length = size;
    memcpy(buf, exe, length);
    return (ssize_t)length;
}

ssize_t readlink(const char* path, char* buf, size_t size) {
    static ssize_t (*real)(const char*, char*, size_t);
    char resolved[PATH_MAX];

    if (real == NULL)
        real = (ssize_t (*)(const char*, char*, size_t))dlsym(RTLD_NEXT, "readlink");
    if (is_self_exe(path)) {
        const char* exe = self_exe(resolved);

        if (exe != NULL)
            return answer(exe, buf, size);
    }
    return real(path, buf, size);
}

ssize_t readlinkat(int dirfd, const char* path, char* buf, size_t size) {
    static ssize_t (*real)(int, const char*, char*, size_t);
    char resolved[PATH_MAX];

    if (real == NULL)
        real = (ssize_t (*)(int, const char*, char*, size_t))dlsym(RTLD_NEXT, "readlinkat");
    if (is_self_exe(path)) {
        const char* exe = self_exe(resolved);

        if (exe != NULL)
            return answer(exe, buf, size);
    }
    return real(dirfd, path, buf, size);
}
