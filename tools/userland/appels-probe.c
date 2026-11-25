/*
 * APPELS-PROBE -- ce que coutent les appels systeme les plus frequents, vus
 * de l'anneau 3 (BOUCHAUD_COUT_APPELS_V1, BOUCHAUD_VDSO_V1).
 *
 *   cout       ns par appel : getpid brut (une transition nue),
 *              clock_gettime MONOTONIC / REALTIME, gettimeofday, time,
 *              sched_getcpu -- par la libc, donc par le vDSO s'il existe
 *   noyau      combien de ces appels ont REELLEMENT atteint le noyau
 *              (/proc/bouchaud/appels, delta autour de la boucle)
 *   monotonie  MONOTONIC ne recule jamais sur N lectures ; REALTIME,
 *              gettimeofday et time concordent (a 1 s pres pour time)
 *   coeurs     deux fils sur deux coeurs se passent un jeton horodate :
 *              la lecture de l'un n'est jamais anterieure a celle de l'autre
 *
 *     APPELS_PROBE_OK              APPELS_PROBE_FAIL raison=...
 *
 * Usage : appels-probe [ITERATIONS]
 */
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

static int echec(const char *r, long long v) { printf("APPELS_PROBE_FAIL raison=%s valeur=%lld\n", r, v); return 1; }

static uint64_t ns(clockid_t c)
{
    struct timespec t;
    clock_gettime(c, &t);
    return (uint64_t)t.tv_sec * 1000000000ull + (uint64_t)t.tv_nsec;
}

/* Appels de `nom` arrives au noyau depuis le demarrage (colonnes purs +
 * commutes de /proc/bouchaud/appels), -1 si le fichier manque (noyau
 * d'avant BOUCHAUD_COUT_APPELS_V1, ou Linux). */
static long long noyau(const char *nom)
{
    FILE *f = fopen("/proc/bouchaud/appels", "r");
    if (!f) return -1;
    char ligne[256], n[64];
    long long purs, total, moyen, pire, commutes, r = 0;
    while (fgets(ligne, sizeof ligne, f))
        if (sscanf(ligne, "%63s %lld %lld %lld %lld %lld", n, &purs, &total, &moyen, &pire, &commutes) == 6
            && !strcmp(n, nom))
            r = purs + commutes;
    fclose(f);
    return r;
}

static volatile long puits;

#define MESURE(etiquette, nom_noyau, expr)                                            \
    do {                                                                              \
        long long k0 = noyau(nom_noyau);                                              \
        uint64_t t0 = ns(CLOCK_MONOTONIC);                                            \
        for (long i = 0; i < n; i++) { expr; }                                        \
        uint64_t t1 = ns(CLOCK_MONOTONIC);                                            \
        long long k1 = noyau(nom_noyau);                                              \
        printf("APPELS_PROBE_COUT appel=%s ns=%llu noyau=%lld n=%ld\n", etiquette,    \
               (unsigned long long)((t1 - t0) / (uint64_t)n),                         \
               k0 < 0 ? -1 : k1 - k0, n);                                             \
    } while (0)

static _Atomic uint64_t jeton;
static _Atomic int tour, fini;
static long allers;
static _Atomic long reculs;

static void *partenaire(void *arg)
{
    (void)arg;
    cpu_set_t c; CPU_ZERO(&c); CPU_SET(1, &c);
    sched_setaffinity(0, sizeof c, &c);
    for (long i = 0; i < allers; i++) {
        while (atomic_load(&tour) != 1) if (atomic_load(&fini)) return NULL;
        uint64_t recu = atomic_load(&jeton), moi = ns(CLOCK_MONOTONIC);
        if (moi < recu) atomic_fetch_add(&reculs, 1);
        atomic_store(&jeton, ns(CLOCK_MONOTONIC));
        atomic_store(&tour, 0);
    }
    return NULL;
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    long n = argc > 1 ? atol(argv[1]) : 200000;
    if (n < 1000) n = 1000;

    MESURE("getpid_brut", "getpid", puits += syscall(SYS_getpid));
    MESURE("clock_gettime_mono", "clock_gettime", { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); puits += t.tv_nsec; });
    MESURE("clock_gettime_real", "clock_gettime", { struct timespec t; clock_gettime(CLOCK_REALTIME, &t); puits += t.tv_nsec; });
    MESURE("gettimeofday", "gettimeofday", { struct timeval t; gettimeofday(&t, 0); puits += t.tv_usec; });
    MESURE("time", "time", puits += time(0));
    MESURE("sched_getcpu", "getcpu", puits += sched_getcpu());
    printf("APPELS_PROBE_ETAPE cout=ok\n");

    /* Monotonie sur une rafale. */
    uint64_t prec = ns(CLOCK_MONOTONIC);
    for (long i = 0; i < n; i++) {
        uint64_t t = ns(CLOCK_MONOTONIC);
        if (t < prec) return echec("monotonic_recule", (long long)(prec - t));
        prec = t;
    }
    printf("APPELS_PROBE_ETAPE monotonie=ok\n");

    /* Concordance des horloges murales. */
    for (int i = 0; i < 1000; i++) {
        struct timespec r; struct timeval v;
        clock_gettime(CLOCK_REALTIME, &r);
        gettimeofday(&v, 0);
        time_t s = time(0);
        long long ecart_us = ((long long)v.tv_sec * 1000000 + v.tv_usec) - ((long long)r.tv_sec * 1000000 + r.tv_nsec / 1000);
        if (ecart_us < 0 || ecart_us > 20000) return echec("gettimeofday_discorde", ecart_us);
        if (s < r.tv_sec || s > r.tv_sec + 1) return echec("time_discorde", (long long)(s - r.tv_sec));
    }
    struct timespec res;
    if (clock_getres(CLOCK_MONOTONIC, &res) != 0 || res.tv_sec != 0 || res.tv_nsec <= 0)
        return echec("clock_getres", res.tv_nsec);
    struct timespec bt;
    if (clock_gettime(CLOCK_BOOTTIME, &bt) != 0) return echec("boottime", errno);
    printf("APPELS_PROBE_ETAPE concordance=ok\n");

    /* Deux coeurs : la lecture d'apres n'est jamais anterieure. */
    long cpus = sysconf(_SC_NPROCESSORS_ONLN);
    if (cpus >= 2) {
        allers = 20000;
        pthread_t p;
        pthread_create(&p, 0, partenaire, 0);
        cpu_set_t c; CPU_ZERO(&c); CPU_SET(0, &c);
        sched_setaffinity(0, sizeof c, &c);
        int sur0 = sched_getcpu();
        atomic_store(&jeton, ns(CLOCK_MONOTONIC));
        for (long i = 0; i < allers; i++) {
            atomic_store(&tour, 1);
            while (atomic_load(&tour) != 0) ;
            uint64_t recu = atomic_load(&jeton), moi = ns(CLOCK_MONOTONIC);
            if (moi < recu) atomic_fetch_add(&reculs, 1);
            atomic_store(&jeton, ns(CLOCK_MONOTONIC));
        }
        atomic_store(&fini, 1);
        pthread_join(p, 0);
        printf("APPELS_PROBE_COEURS allers=%ld reculs=%ld cpu_principal=%d\n", allers, atomic_load(&reculs), sur0);
        if (atomic_load(&reculs)) return echec("recul_entre_coeurs", atomic_load(&reculs));
        printf("APPELS_PROBE_ETAPE coeurs=ok\n");
    }
    printf("APPELS_PROBE_OK\n");
    return 0;
}
