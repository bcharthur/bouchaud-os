#define _GNU_SOURCE
#include <poll.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>

static int wake_pipe[2];
static volatile int stop;
static uint64_t cpu_iterations;
static uint64_t wakeups;
static uint64_t latency_sum_ns;
static uint64_t latency_max_ns;

static uint64_t ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static void *cpu_worker(void *unused) {
    (void)unused;
    uint64_t x = 1;
    while (!stop) {
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        ++cpu_iterations;
    }
    return (void *)(uintptr_t)x;
}

static void *sleeper(void *unused) {
    (void)unused;
    struct pollfd fd = { .fd = wake_pipe[0], .events = POLLIN };
    while (!stop) {
        if (poll(&fd, 1, 100) <= 0) continue;
        uint64_t sent;
        if (read(wake_pipe[0], &sent, sizeof(sent)) != sizeof(sent)) continue;
        uint64_t latency = ns() - sent;
        latency_sum_ns += latency;
        if (latency > latency_max_ns) latency_max_ns = latency;
        ++wakeups;
    }
    return NULL;
}

int main(void) {
    pthread_t cpu, wait;
    if (pipe(wake_pipe) != 0) return 2;
    pthread_create(&cpu, NULL, cpu_worker, NULL);
    pthread_create(&wait, NULL, sleeper, NULL);
    uint64_t start = ns();
    for (unsigned i = 0; i < 200; ++i) {
        struct timespec delay = { .tv_nsec = 10000000 };
        nanosleep(&delay, NULL);
        uint64_t sent = ns();
        write(wake_pipe[1], &sent, sizeof(sent));
    }
    stop = 1;
    pthread_join(cpu, NULL);
    pthread_join(wait, NULL);
    printf("SMPMIX wall_ms=%llu cpu_iterations=%llu wakeups=%llu latency_avg_us=%llu latency_max_us=%llu\n",
        (unsigned long long)((ns() - start) / 1000000),
        (unsigned long long)cpu_iterations, (unsigned long long)wakeups,
        (unsigned long long)(wakeups ? latency_sum_ns / wakeups / 1000 : 0),
        (unsigned long long)(latency_max_ns / 1000));
    return 0;
}
