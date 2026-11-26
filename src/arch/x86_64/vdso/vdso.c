/*
 * BOUCHAUD_VDSO_V1 -- l'heure et le coeur lus en anneau 3, sans appel systeme.
 *
 * Projete par le noyau dans chaque processus (AT_SYSINFO_EHDR), juste apres sa
 * page de donnees `vvar` (lecture seule, partagee, ecrite par le noyau seul).
 * La glibc et musl y cherchent `__vdso_clock_gettime` & co. version LINUX_2.6.
 *
 * Le calcul est EXACTEMENT celui du noyau (`timer::tsc_vers_ns`) :
 *     ns = (c / hz) * 1e9 + ((c % hz) * 1e9) / hz,   c = TSC - TSC du boot
 * un appel systeme et une lecture ici, au meme TSC, rendent le meme nombre.
 *
 * Le chemin rapide n'est pris que si le noyau l'a publie (mode TSC) : TSC
 * prouve synchrone entre coeurs, frequence connue. Sinon, ou pour toute
 * horloge non traitee ici (temps processeur, horloges inconnues), l'appel
 * systeme -- jamais une valeur inventee.
 *
 * Aucune donnee ecrite, aucune relocation, aucune dependance : ce code est
 * partage par tous les processus et ne doit rien supposer d'eux.
 */
#include <stdint.h>

struct bo_vvar {
    volatile uint32_t seq;          /* impair : publication en cours */
    volatile uint32_t mode;         /* 0 : appel systeme ; 1 : TSC */
    volatile uint64_t boot_tsc;
    volatile uint64_t hz;
    volatile uint32_t rdtscp;       /* 1 : rdtscp, sinon lfence ; rdtsc */
    volatile uint32_t getcpu;       /* 0 : appel ; 1 : rdtscp ; 2 : rdpid */
    volatile int64_t realtime_offset_ns;
    volatile uint32_t realtime_ok;
    volatile uint32_t aux_etiquette;
};

/* La page vvar precede le texte du vDSO (vdso.lds). */
extern const struct bo_vvar bo_vvar_page __attribute__((visibility("hidden")));

struct ts { int64_t tv_sec; int64_t tv_nsec; };
struct tv { int64_t tv_sec; int64_t tv_usec; };
struct tz { int32_t minuteswest; int32_t dsttime; };

#define NR_GETTIMEOFDAY 96
#define NR_TIME 201
#define NR_CLOCK_GETTIME 228
#define NR_GETCPU 309

static inline long sys2(long n, long a, long b)
{
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b) : "rcx", "r11", "memory");
    return r;
}

static inline long sys3(long n, long a, long b, long c)
{
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return r;
}

#define barriere() __asm__ volatile("" ::: "memory")

static inline uint64_t tsc(uint32_t rdtscp)
{
    uint32_t lo, hi;
    if (rdtscp)
        __asm__ volatile("rdtscp" : "=a"(lo), "=d"(hi) :: "rcx", "memory");
    else
        __asm__ volatile("lfence; rdtsc" : "=a"(lo), "=d"(hi) :: "memory");
    return ((uint64_t)hi << 32) | lo;
}

/* Horloge monotone du noyau, en ns ; 0 si le chemin rapide est ferme. Avec le
 * decalage mural si `mural` (0 s'il n'est pas encore pose). */
static inline int lit(int mural, uint64_t *ns)
{
    const struct bo_vvar *v = &bo_vvar_page;
    uint32_t s;
    uint64_t c, hz, boot;
    int64_t decalage;
    do {
        s = v->seq;
        barriere();
        if (s & 1) continue;
        if (v->mode != 1) return 0;
        if (mural && !v->realtime_ok) return 0;
        hz = v->hz;
        boot = v->boot_tsc;
        decalage = v->realtime_offset_ns;
        c = tsc(v->rdtscp) - boot;
        barriere();
    } while (v->seq != s || (s & 1));
    if (hz == 0) return 0;
    uint64_t r = (c / hz) * 1000000000ull + ((c % hz) * 1000000000ull) / hz;
    if (mural) r += (uint64_t)decalage;
    *ns = r;
    return 1;
}

int __vdso_clock_gettime(int horloge, struct ts *t)
{
    int mural;
    switch (horloge) {
    case 0: case 5: case 8: mural = 1; break;          /* REALTIME, _COARSE, _ALARM */
    case 1: case 4: case 6: case 7: case 9: mural = 0; break; /* MONOTONIC, _RAW, _COARSE, BOOTTIME, _ALARM */
    default: return (int)sys2(NR_CLOCK_GETTIME, horloge, (long)t);
    }
    uint64_t ns;
    if (!lit(mural, &ns)) return (int)sys2(NR_CLOCK_GETTIME, horloge, (long)t);
    t->tv_sec = (int64_t)(ns / 1000000000ull);
    t->tv_nsec = (int64_t)(ns % 1000000000ull);
    return 0;
}

int __vdso_gettimeofday(struct tv *t, struct tz *z)
{
    uint64_t ns;
    if (t && !lit(1, &ns)) return (int)sys2(NR_GETTIMEOFDAY, (long)t, (long)z);
    if (t) {
        t->tv_sec = (int64_t)(ns / 1000000000ull);
        t->tv_usec = (int64_t)((ns % 1000000000ull) / 1000);
    }
    if (z) { z->minuteswest = 0; z->dsttime = 0; }
    return 0;
}

int64_t __vdso_time(int64_t *t)
{
    uint64_t ns;
    if (!lit(1, &ns)) return sys2(NR_TIME, (long)t, 0);
    int64_t s = (int64_t)(ns / 1000000000ull);
    if (t) *t = s;
    return s;
}

int __vdso_getcpu(uint32_t *cpu, uint32_t *node, void *cache)
{
    const struct bo_vvar *v = &bo_vvar_page;
    uint32_t mode = v->getcpu, aux = 0;
    if (mode == 2) {
        uint64_t r;
        __asm__ volatile("rdpid %0" : "=r"(r));
        aux = (uint32_t)r;
    } else if (mode == 1) {
        uint32_t lo, hi;
        __asm__ volatile("rdtscp" : "=a"(lo), "=d"(hi), "=c"(aux));
    }
    /* Une etiquette absente (TSC_AUX d'un coeur pas encore verifie) : le
     * noyau tranche. */
    if (mode == 0 || (aux & 0xFFFF0000u) != v->aux_etiquette || (aux & 0xFFFFu) == 0)
        return (int)sys3(NR_GETCPU, (long)cpu, (long)node, (long)cache);
    if (cpu) *cpu = (aux & 0xFFFFu) - 1;
    if (node) *node = 0;
    return 0;
}

int clock_gettime(int, struct ts *) __attribute__((weak, alias("__vdso_clock_gettime")));
int gettimeofday(struct tv *, struct tz *) __attribute__((weak, alias("__vdso_gettimeofday")));
int64_t time(int64_t *) __attribute__((weak, alias("__vdso_time")));
int getcpu(uint32_t *, uint32_t *, void *) __attribute__((weak, alias("__vdso_getcpu")));
