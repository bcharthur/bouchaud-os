/* Periodic ring-3 export of the system and one optional process snapshot. */
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static int system_sample(void)
{
    FILE *file = fopen("/proc/bouchaud/metrics", "r");
    if (!file) {
        perror("/proc/bouchaud/metrics");
        return -1;
    }

    char buffer[1024];
    while (fgets(buffer, sizeof buffer, file))
        if (fputs(buffer, stdout) == EOF)
            break;
    int failed = ferror(file) || ferror(stdout);
    fclose(file);
    return failed ? -1 : 0;
}

static int process_sample(long pid, const struct timespec *time)
{
    char path[64];
    snprintf(path, sizeof path, "/proc/%ld/stat", pid);
    FILE *file = fopen(path, "r");
    if (!file) {
        perror(path);
        return -1;
    }

    char *record = NULL;
    size_t capacity = 0;
    ssize_t length = getline(&record, &capacity, file);
    fclose(file);
    /* comm is parenthesized and can contain whitespace and parentheses. */
    char *end_name = length > 0 ? strrchr(record, ')') : NULL;
    unsigned long long user, system;
    long long rss;
    int fields = end_name ? sscanf(end_name + 1,
        " %*c %*s %*s %*s %*s %*s %*s %*s %*s %*s %*s %llu %llu"
        " %*s %*s %*s %*s %*s %*s %*s %*s %lld", &user, &system, &rss) : 0;
    free(record);
    if (fields != 3 || rss < 0) {
        fprintf(stderr, "PROCESS_METRICS invalid_stat pid=%ld\n", pid);
        return -1;
    }

    return printf("PROCESS_METRICS monotonic_ns=%llu pid=%ld user_ticks=%llu system_ticks=%llu"
                  " rss_pages=%lld io_read=N/A io_write=N/A\n",
                  (unsigned long long)time->tv_sec * 1000000000ULL + time->tv_nsec,
                  pid, user, system, rss) < 0 ? -1 : 0;
}

static long positive(const char *argument)
{
    char *end;
    errno = 0;
    long value = strtol(argument, &end, 10);
    return errno || *end || value < 1 ? 0 : value;
}

int main(int argc, char **argv)
{
    long samples = argc > 1 ? positive(argv[1]) : 1;
    long pid = argc > 2 ? positive(argv[2]) : getpid();
    if (!samples || !pid || argc > 3) {
        fprintf(stderr, "usage: collecte-metriques [samples [pid]]\n");
        return 2;
    }

    for (long sample = 0; sample < samples; ++sample) {
        struct timespec now;
        if (clock_gettime(CLOCK_MONOTONIC, &now) < 0 || system_sample() < 0
            || process_sample(pid, &now) < 0 || fflush(stdout) == EOF)
            return 1;
        if (sample + 1 < samples) {
            struct timespec delay = { .tv_sec = 1 };
            while (nanosleep(&delay, &delay) < 0)
                if (errno != EINTR)
                    return 1;
        }
    }
    return 0;
}
