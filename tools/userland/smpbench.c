#define _GNU_SOURCE
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

struct worker { unsigned id; uint64_t iterations; uint64_t checksum; };

static uint64_t ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static void *run(void *opaque) {
    struct worker *w = opaque;
    uint64_t x = 0x9e3779b97f4a7c15ull ^ w->id;
    const uint64_t end = ns() + 2000000000ull;
    while (ns() < end) {
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        w->iterations++;
    }
    w->checksum = x;
    return NULL;
}

int main(int argc, char **argv) {
    unsigned count = argc > 1 ? (unsigned)strtoul(argv[1], NULL, 10) : 1;
    if (count < 1 || count > 64) { fprintf(stderr, "usage: smpbench 1..64\n"); return 2; }
    pthread_t *threads = calloc(count, sizeof(*threads));
    struct worker *workers = calloc(count, sizeof(*workers));
    uint64_t start = ns();
    for (unsigned i = 0; i < count; ++i) { workers[i].id = i; pthread_create(&threads[i], NULL, run, &workers[i]); }
    for (unsigned i = 0; i < count; ++i) pthread_join(threads[i], NULL);
    uint64_t iterations = 0, checksum = 0;
    for (unsigned i = 0; i < count; ++i) { iterations += workers[i].iterations; checksum ^= workers[i].checksum; }
    printf("SMPBENCH workers=%u wall_ms=%llu iterations=%llu checksum=%llx\n", count,
        (unsigned long long)((ns() - start) / 1000000), (unsigned long long)iterations,
        (unsigned long long)checksum);
    return 0;
}
