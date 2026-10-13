/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Bounded by the systemd unit that launches it; run only in a disposable VM. */
#include <errno.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

static volatile sig_atomic_t running = 1;
static pid_t children[3];

static void stop(int signal_number) {
    (void)signal_number;
    running = 0;
}

static int run_cpu(void) {
    for (size_t index = 0; index < 3; index++) {
        children[index] = fork();
        if (children[index] < 0)
            return 5;
        if (children[index] == 0)
            break;
    }
    volatile uint64_t value = 1;
    while (running)
        value = value * UINT64_C(6364136223846793005) + 1;
    return value == 0;
}

static int run_memory(const char* argument) {
    char* end = NULL;
    errno = 0;
    unsigned long long mib = strtoull(argument, &end, 10);
    if (errno || !argument[0] || *end || mib == 0 || mib > SIZE_MAX / (1024 * 1024))
        return 3;
    size_t size = (size_t)mib * 1024 * 1024;
    volatile unsigned char* memory =
        mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (memory == MAP_FAILED)
        return 4;
    uint64_t seed = 17;
    for (size_t offset = 0; offset < size && running; offset += sizeof(seed)) {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        *(volatile uint64_t*)(memory + offset) = seed;
        if (offset % (16 * 1024 * 1024) == 0)
            usleep(50000);
    }
    while (running) {
        for (size_t offset = 0; offset < size; offset += 4096)
            memory[offset]++;
        usleep(50000);
    }
    munmap((void*)memory, size);
    return 0;
}

int main(int argc, char** argv) {
    if (argc < 2 || argc > 3) {
        fprintf(stderr, "usage: %s cpu | memory MIB\n", argv[0]);
        return 2;
    }
    if (signal(SIGTERM, stop) == SIG_ERR || signal(SIGINT, stop) == SIG_ERR)
        return 6;
    if (strcmp(argv[1], "cpu") == 0 && argc == 2)
        return run_cpu();
    if (strcmp(argv[1], "memory") == 0 && argc == 3)
        return run_memory(argv[2]);
    fprintf(stderr, "usage: %s cpu | memory MIB\n", argv[0]);
    return 2;
}
