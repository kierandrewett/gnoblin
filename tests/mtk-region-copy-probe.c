/* Count MtkRegion copy call sites in a running compositor.
 *
 * Build with scripts/build-mtk-region-copy-probe.sh and preload into a private
 * test compositor. Each result line gives the caller module and offset; use
 * addr2line on the matching unstripped library to resolve the source line.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

typedef struct _MtkRegion MtkRegion;
typedef MtkRegion* (*MtkRegionCopyFunc)(const MtkRegion* region);

typedef struct {
    const void* base;
    uintptr_t offset;
    unsigned long count;
    const char* path;
} CopySite;

#define MAX_SITES 128

static CopySite sites[MAX_SITES];
static unsigned int n_sites;
static pthread_mutex_t sites_lock = PTHREAD_MUTEX_INITIALIZER;

static void record_copy_site(void* return_address) {
    Dl_info info;
    uintptr_t offset;

    if (!dladdr(return_address, &info) || !info.dli_fbase || !info.dli_fname)
        return;

    offset = (uintptr_t)return_address - (uintptr_t)info.dli_fbase;

    pthread_mutex_lock(&sites_lock);
    for (unsigned int i = 0; i < n_sites; i++) {
        if (sites[i].base == info.dli_fbase && sites[i].offset == offset) {
            sites[i].count++;
            pthread_mutex_unlock(&sites_lock);
            return;
        }
    }

    if (n_sites < MAX_SITES) {
        sites[n_sites++] = (CopySite){
            .base = info.dli_fbase,
            .offset = offset,
            .count = 1,
            .path = info.dli_fname,
        };
    }
    pthread_mutex_unlock(&sites_lock);
}

MtkRegion* mtk_region_copy(const MtkRegion* region) {
    static MtkRegionCopyFunc real_copy;

    if (!real_copy)
        real_copy = (MtkRegionCopyFunc)dlsym(RTLD_NEXT, "mtk_region_copy");

    record_copy_site(__builtin_return_address(0));
    return real_copy(region);
}

__attribute__((destructor)) static void write_copy_sites(void) {
    const char* path = getenv("GNOBLIN_MTK_REGION_COPY_LOG");
    int fd;

    if (!path)
        return;

    fd = open(path, O_WRONLY | O_CREAT | O_APPEND, 0600);
    if (fd < 0)
        return;

    for (unsigned int i = 0; i < n_sites; i++)
        dprintf(fd, "MTK_REGION_COPY path=%s offset=0x%lx count=%lu\n", sites[i].path,
                (unsigned long)sites[i].offset, sites[i].count);

    close(fd);
}
