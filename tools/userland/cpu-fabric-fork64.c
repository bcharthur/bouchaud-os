#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static uint64_t ns(void)
{
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t) != 0)
        _exit(2);
    return (uint64_t)t.tv_sec * 1000000000 + t.tv_nsec;
}

int main(void)
{
    const size_t taille = 64UL << 20;
    volatile unsigned char *pages = mmap(NULL, taille, PROT_READ | PROT_WRITE,
                                         MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (pages == MAP_FAILED)
        return 1;
    for (size_t i = 0; i < taille; i += 4096)
        pages[i] = 1;
    for (int i = 0; i < 20; ++i) {
        int statut;
        const uint64_t debut = ns();
        const pid_t fils = fork();
        if (fils == 0)
            _exit(pages[taille - 4096] != 1);
        const uint64_t duree = ns() - debut;
        if (fils < 0 || waitpid(fils, &statut, 0) != fils ||
            !WIFEXITED(statut) || WEXITSTATUS(statut))
            return 3;
        printf("FORK64_SAMPLE i=%d ns=%llu\n", i, (unsigned long long)duree);
    }
    puts("FORK64_OK");
    return 0;
}
