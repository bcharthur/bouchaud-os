#define _GNU_SOURCE
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <time.h>

struct worker { size_t pages; uint64_t checksum; };

static void *run(void *opaque) {
    struct worker *w = opaque;
    size_t bytes = w->pages * 4096;
    unsigned char *p = mmap(NULL, bytes, PROT_READ | PROT_WRITE,
                            MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) return (void *)1;
    uint64_t sum = 0;
    for (size_t i = 0; i < w->pages; ++i) {
        p[i * 4096] = (unsigned char)(i * 131u);
        sum += p[i * 4096];
    }
    w->checksum = sum;
    return (void *)(uintptr_t)(munmap(p, bytes) != 0);
}

int main(int argc, char **argv) {
    int workers = argc > 1 ? atoi(argv[1]) : 1;
    size_t pages = argc > 2 ? strtoull(argv[2], NULL, 10) : 4096;
    if (workers < 1 || workers > 64 || pages < 1) return 2;
    pthread_t *threads = calloc((size_t)workers, sizeof(*threads));
    struct worker *jobs = calloc((size_t)workers, sizeof(*jobs));
    struct timespec a, b;
    clock_gettime(CLOCK_MONOTONIC, &a);
    for (int i = 0; i < workers; ++i) {
        jobs[i].pages = pages;
        if (pthread_create(&threads[i], NULL, run, &jobs[i])) return 3;
    }
    uint64_t checksum = 0;
    for (int i = 0; i < workers; ++i) {
        void *result;
        pthread_join(threads[i], &result);
        if (result) return 4;
        checksum += jobs[i].checksum;
    }
    clock_gettime(CLOCK_MONOTONIC, &b);
    uint64_t ns = (uint64_t)(b.tv_sec - a.tv_sec) * 1000000000ull
                + (uint64_t)(b.tv_nsec - a.tv_nsec);
    printf("mmstress workers=%d pages=%zu wall_ms=%llu checksum=%llu\n",
           workers, pages, (unsigned long long)(ns / 1000000),
           (unsigned long long)checksum);
    free(jobs); free(threads);
    return 0;
}
