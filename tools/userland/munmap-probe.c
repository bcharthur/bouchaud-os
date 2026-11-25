/*
 * BOUCHAUD_DEMONTAGE_PAR_TABLES_V1 -- duree d'un munmap / d'une reduction de
 * brk sur une plage GRANDE et PEU PEUPLEE (le cas des reservations de tas et
 * de GC de Ladybird : WebContent porte ~8 Tio de VSS pour ~100 Mio de RSS).
 *
 *   munmap-probe [taille_mio] [pages_touchees] [repetitions]
 *
 * Pour chaque repetition : mmap anonyme de `taille_mio`, une page touchee
 * tous les taille/pages_touchees octets, puis munmap chronometre. Ensuite
 * le meme schema sur le tas (sbrk +taille, pages touchees, sbrk -taille).
 * Sortie : une ligne MUNMAP_PROBE par mesure, puis le pire et la mediane.
 */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

static uint64_t maintenant_us(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000u + (uint64_t)ts.tv_nsec / 1000u;
}

static int compare(const void *a, const void *b)
{
    uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b;
    return x < y ? -1 : x > y;
}

static void bilan(const char *nom, uint64_t *v, int n)
{
    qsort(v, (size_t)n, sizeof *v, compare);
    printf("MUNMAP_PROBE_BILAN op=%s n=%d mediane_us=%llu pire_us=%llu\n", nom, n,
        (unsigned long long)v[n / 2], (unsigned long long)v[n - 1]);
}

int main(int argc, char **argv)
{
    uint64_t taille = (argc > 1 ? strtoull(argv[1], 0, 10) : 1024) << 20;
    uint64_t touchees = argc > 2 ? strtoull(argv[2], 0, 10) : 8;
    int reps = argc > 3 ? atoi(argv[3]) : 5;
    uint64_t pas = touchees ? taille / touchees : taille;
    uint64_t mm[64], br[64];
    if (reps > 64)
        reps = 64;

    for (int r = 0; r < reps; ++r) {
        char *p = mmap(0, taille, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
        if (p == MAP_FAILED) {
            printf("MUNMAP_PROBE_ECHEC op=mmap\n");
            return 1;
        }
        for (uint64_t o = 0; o < taille; o += pas)
            p[o] = 1;
        uint64_t t0 = maintenant_us();
        if (munmap(p, taille) != 0) {
            printf("MUNMAP_PROBE_ECHEC op=munmap\n");
            return 1;
        }
        mm[r] = maintenant_us() - t0;
        printf("MUNMAP_PROBE op=munmap taille_mio=%llu touchees=%llu us=%llu\n",
            (unsigned long long)(taille >> 20), (unsigned long long)touchees, (unsigned long long)mm[r]);
    }
    bilan("munmap", mm, reps);
    /* Le tas est borne par la limite du processus : 64 Mio au plus. */
    uint64_t tas = taille > (64ull << 20) ? (64ull << 20) : taille;
    uint64_t pas_tas = touchees ? tas / touchees : tas;
    for (int r = 0; r < reps; ++r) {
        /* musl refuse sbrk(n != 0) : l'appel systeme brk directement. */
        char *base = (char *)syscall(SYS_brk, 0);
        if ((char *)syscall(SYS_brk, base + tas) != base + tas) {
            printf("MUNMAP_PROBE_ECHEC op=brk_grow\n");
            return 1;
        }
        for (uint64_t o = 0; o < tas; o += pas_tas)
            base[o] = 1;
        uint64_t t0 = maintenant_us();
        if ((char *)syscall(SYS_brk, base) != base) {
            printf("MUNMAP_PROBE_ECHEC op=brk_shrink\n");
            return 1;
        }
        br[r] = maintenant_us() - t0;
        printf("MUNMAP_PROBE op=brk_shrink taille_mio=%llu touchees=%llu us=%llu\n",
            (unsigned long long)(tas >> 20), (unsigned long long)touchees, (unsigned long long)br[r]);
    }
    bilan("brk_shrink", br, reps);
    printf("MUNMAP_PROBE_OK\n");
    return 0;
}
