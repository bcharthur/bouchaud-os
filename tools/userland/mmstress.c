#define _GNU_SOURCE
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

struct worker { size_t pages; unsigned rounds; uint64_t checksum; };

static void *run(void *opaque) {
    struct worker *w = opaque;
    uint64_t sum = 0;
    size_t bytes = w->pages * 4096;
    for (unsigned round = 0; round < w->rounds; ++round) {
        unsigned char *p = mmap(NULL, bytes, PROT_READ | PROT_WRITE,
                                MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (p == MAP_FAILED) return (void *)1;
        for (size_t i = 0; i < w->pages; ++i) {
            p[i * 4096] = (unsigned char)(i * 131u + round);
            sum += p[i * 4096];
        }
        if (mprotect(p, bytes, PROT_READ) || mprotect(p, bytes, PROT_READ | PROT_WRITE))
            return (void *)2;
        if (munmap(p, bytes)) return (void *)3;
    }
    w->checksum = sum;
    return NULL;
}

int main(int argc, char **argv) {
    int workers = argc > 1 ? atoi(argv[1]) : 1;
    size_t pages = argc > 2 ? strtoull(argv[2], NULL, 10) : 4096;
    unsigned rounds = argc > 3 ? (unsigned)strtoul(argv[3], NULL, 10) : 8;
    if (workers < 1 || workers > 64 || pages < 1 || rounds < 1) return 2;
    pthread_t *threads = calloc((size_t)workers, sizeof(*threads));
    struct worker *jobs = calloc((size_t)workers, sizeof(*jobs));
    struct timespec a, b;
    clock_gettime(CLOCK_MONOTONIC, &a);
    for (int i = 0; i < workers; ++i) {
        jobs[i].pages = pages;
        jobs[i].rounds = rounds;
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
    pid_t child = fork();
    if (child == 0) {
        void *p = mmap(NULL, 4096, PROT_READ | PROT_WRITE,
                       MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        _exit(p == MAP_FAILED || munmap(p, 4096) ? 5 : 0);
    }
    int status = 0;
    if (child < 0 || waitpid(child, &status, 0) != child || !WIFEXITED(status)
        || WEXITSTATUS(status) != 0) return 5;
    uint64_t ns = (uint64_t)(b.tv_sec - a.tv_sec) * 1000000000ull
                + (uint64_t)(b.tv_nsec - a.tv_nsec);
    printf("mmstress workers=%d pages=%zu rounds=%u wall_ms=%llu checksum=%llu\n",
           workers, pages, rounds, (unsigned long long)(ns / 1000000),
           (unsigned long long)checksum);
    free(jobs); free(threads);
    return 0;
}
