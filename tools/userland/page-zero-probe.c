/* Allocate, first-touch, and release 32 MiB per trial. The odd stride
 * permutes all pages while avoiding the sequential fault-cluster path.
 * Each page is checked before writing, including across repeated unmaps.
 * Elapsed time covers the first touches. The guest's fautes command
 * reports the corresponding kernel fault counts and cumulative latency. */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <time.h>

static uint64_t clock_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000 + ts.tv_nsec / 1000;
}

int main(void) {
    enum { page_size = 4096, pages = 8192 };
    for (int trial = 0; trial < 24; trial++) {
        volatile unsigned char *mapping = mmap(NULL, pages * page_size,
            PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (mapping == MAP_FAILED) return 1;
        uint64_t start = clock_us();
        for (unsigned int i = 0; i < pages; i++) {
            unsigned int offset = ((i * 3691) % pages) * page_size;
            if (mapping[offset] != 0) return 2;
            mapping[offset] = 0x59;
        }
        printf("PAGE_ZERO trial=%d elapsed_us=%llu pages=%d\n", trial,
               (unsigned long long)(clock_us() - start), pages);
        if (munmap((void *)mapping, pages * page_size) != 0) return 3;
    }
    puts("PAGE_ZERO_OK");
    return 0;
}
