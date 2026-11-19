/* Repeated fsync on stable sector-aligned and unaligned files.
 * One changed file exercises reuse and still requires a durable commit. */
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>

static uint64_t monotone_us(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000 + ts.tv_nsec / 1000;
}

int main(void)
{
    static char bloc[4096];
    bloc[0] = 'A';
    int grand = open("/persist/ab-grand", O_RDWR | O_CREAT | O_TRUNC, 0600);
    int petit = open("/persist/ab-petit", O_RDWR | O_CREAT | O_TRUNC, 0600);
    if (grand < 0 || petit < 0)
        return 1;
    for (int i = 0; i < 768; ++i) {
        if (write(grand, bloc, sizeof(bloc)) != sizeof(bloc))
            return 2;
    }
    if (write(petit, "AB", 2) != 2)
        return 3;
    for (int i = 0; i < 8; ++i) {
        char version = 'a' + i;
        if (pwrite(petit, &version, 1, 1) != 1)
            return 4;
        uint64_t debut = monotone_us();
        if (fsync(petit))
            return 5;
        printf("PERSIST_REPETITION cycle=%d elapsed_us=%llu\n", i,
               (unsigned long long)(monotone_us() - debut));
    }
    puts("PERSIST_REPETITION_OK");
    return 0;
}
