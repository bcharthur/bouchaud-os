#define _GNU_SOURCE
#include <fcntl.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
#include <string.h>

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

struct race_case {
    unsigned char *address;
    int fd;
    unsigned rounds;
    volatile int start;
    uint64_t checksum;
};

static void *fault_reader(void *opaque) {
    struct race_case *c = opaque;
    while (!c->start) sched_yield();
    uint64_t sum = 0;
    for (unsigned i = 0; i < c->rounds; ++i) {
        madvise(c->address, 4096, MADV_DONTNEED);
        sum += c->address[(i * 127u) & 4095u];
    }
    c->checksum = sum;
    return NULL;
}

static int race_mode(const char *mode, const char *self) {
    int fd = open(self, O_RDONLY);
    if (fd < 0) return 10;
    unsigned char *mapped = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, 0);
    if (mapped == MAP_FAILED) return 11;
    struct race_case c = { mapped, fd, 2000, 0, 0 };
    pthread_t reader;
    if (pthread_create(&reader, NULL, fault_reader, &c)) return 12;
    c.start = 1;
    for (unsigned i = 0; i < c.rounds; ++i) {
        if (!strcmp(mode, "unrelated")) {
            unsigned char *other = mmap(NULL, 8192, PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            if (other == MAP_FAILED) return 13;
            other[4096] = (unsigned char)i;
            if (mprotect(other, 4096, PROT_READ) || munmap(other, 8192)) return 14;
        } else {
            void *replacement = mmap(mapped, 4096, PROT_READ,
                MAP_PRIVATE | MAP_FIXED, fd, 0);
            if (replacement != mapped) return 15;
        }
    }
    void *result = NULL;
    pthread_join(reader, &result);
    if (result || munmap(mapped, 4096) || close(fd)) return 16;
    printf("MMSTRESS_%s_DONE rounds=%u checksum=%llu\n",
        !strcmp(mode, "aba") ? "ABA" : "UNRELATED", c.rounds,
        (unsigned long long)c.checksum);
    return 0;
}

int main(int argc, char **argv) {
    if (argc > 1 && (!strcmp(argv[1], "unrelated") || !strcmp(argv[1], "aba")))
        return race_mode(argv[1], argv[0]);
    int churn = argc > 1 && !strcmp(argv[1], "churn");
    int workers = churn ? 4 : (argc > 1 ? atoi(argv[1]) : 1);
    size_t pages = churn ? 512 : (argc > 2 ? strtoull(argv[2], NULL, 10) : 4096);
    unsigned rounds = churn ? 128 : (argc > 3 ? (unsigned)strtoul(argv[3], NULL, 10) : 8);
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
    if (churn)
        printf("MMSTRESS_CHURN_OK workers=%d rounds=%u checksum=%llu failures=0\n",
               workers, rounds, (unsigned long long)checksum);
    free(jobs); free(threads);
    return 0;
}
