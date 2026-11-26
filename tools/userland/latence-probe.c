/*
 * LATENCE-PROBE -- le bureau reste-t-il reactif quand le systeme travaille ?
 * (BOUCHAUD_LATENCE_INTERACTIVE_V1)
 *
 * Un processus INTERACTIF (nice -5 : classe interactive) dort 1 ms en boucle
 * (clock_nanosleep absolu) et mesure de combien chaque reveil est en retard --
 * l'equivalent, vu de l'anneau 3, du fil d'entree du bureau. Pendant ce temps,
 * des perturbateurs NORMAUX tournent, une phase chacun :
 *
 *   repos     rien
 *   calcul    deux boucles de calcul pur (le bureau doit les preempter)
 *   fork      un processus de 64 Mio forke en rafale (fils qui sortent)
 *   munmap    reserve 4 Gio, touche 8 pages, demappe, en boucle
 *   journal   ecrit des lignes sur la console (port serie)
 *   spawn     posix_spawn de soi-meme (fork+exec court), en rafale
 *
 * Par phase : LATENCE_PROBE phase= n= p50_us= p95_us= p99_us= max_us=
 * sup_10ms= sup_100ms=
 *
 * Usage : latence-probe [SECONDES_PAR_PHASE [PHASE [NICE]]]
 *         (defaut 3 s, toutes les phases, nice -5 pour le mesureur)
 *         latence-probe rien                      (fils de spawn)
 */
#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>
#include <spawn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

static uint64_t ns(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (uint64_t)t.tv_sec * 1000000000ull + (uint64_t)t.tv_nsec;
}

static int cmp(const void *a, const void *b)
{
    uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b;
    return x < y ? -1 : x > y;
}

static void perturbe(const char *phase, const char *soi)
{
    setpriority(PRIO_PROCESS, 0, 0);   /* normal, pas interactif */
    if (!strcmp(phase, "calcul")) {
        volatile uint64_t x = 1;
        for (;;) x = x * 6364136223846793005ull + 1;
    }
    if (!strcmp(phase, "fork")) {
        size_t n = 64u << 20;
        char *p = malloc(n);
        for (size_t i = 0; i < n; i += 4096) p[i] = 1;
        for (;;) {
            pid_t f = fork();
            if (f == 0) _exit(0);
            if (f > 0) waitpid(f, 0, 0);
        }
    }
    if (!strcmp(phase, "munmap")) {
        for (;;) {
            char *p = mmap(0, 4ull << 30, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
            if (p == MAP_FAILED) continue;
            for (int i = 0; i < 8; i++) p[(size_t)i * (512ull << 20)] = 1;
            munmap(p, 4ull << 30);
        }
    }
    if (!strcmp(phase, "journal")) {
        char ligne[160];
        memset(ligne, 'j', sizeof ligne - 2);
        ligne[sizeof ligne - 2] = '\n';
        ligne[sizeof ligne - 1] = 0;
        for (;;) fputs(ligne, stdout), fflush(stdout);
    }
    if (!strcmp(phase, "spawn")) {
        char *argv[] = { (char *)soi, "rien", 0 };
        for (;;) {
            pid_t f;
            if (posix_spawn(&f, soi, 0, 0, argv, environ) == 0) waitpid(f, 0, 0);
        }
    }
    for (;;) pause();
}

static void mesure(const char *phase, int secondes, const char *soi)
{
    int nb = !strcmp(phase, "calcul") ? 2 : !strcmp(phase, "repos") ? 0 : 1;
    pid_t p[2];
    for (int i = 0; i < nb; i++) {
        p[i] = fork();
        if (p[i] < 0) { perror("fork perturbateur"); exit(1); }
        if (p[i] == 0) perturbe(phase, soi);
    }
    usleep(200000);
    size_t cap = (size_t)secondes * 1200 + 16, n = 0;
    uint64_t *ret = malloc(cap * sizeof *ret);
    uint64_t fin = ns() + (uint64_t)secondes * 1000000000ull;
    uint64_t cible = ns();
    while (n < cap) {
        cible += 1000000;
        uint64_t maintenant = ns();
        if (cible < maintenant) cible = maintenant + 1000000;
        struct timespec t = { (time_t)(cible / 1000000000ull), (long)(cible % 1000000000ull) };
        while (clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &t, 0) == EINTR) ;
        uint64_t r = ns();
        ret[n++] = r > cible ? r - cible : 0;
        if (r >= fin) break;
    }
    for (int i = 0; i < nb; i++) {
        int etat;
        if (kill(p[i], SIGKILL) != 0 || waitpid(p[i], &etat, 0) != p[i]
            || !WIFSIGNALED(etat) || WTERMSIG(etat) != SIGKILL) {
            fprintf(stderr, "LATENCE_PROBE_FAIL phase=%s sigkill pid=%ld\n", phase, (long)p[i]);
            exit(1);
        }
    }
    qsort(ret, n, sizeof *ret, cmp);
    size_t s10 = 0, s100 = 0;
    for (size_t i = 0; i < n; i++) { s10 += ret[i] > 10000000; s100 += ret[i] > 100000000; }
    printf("LATENCE_PROBE phase=%s n=%zu p50_us=%llu p95_us=%llu p99_us=%llu max_us=%llu sup_10ms=%zu sup_100ms=%zu\n",
           phase, n,
           (unsigned long long)ret[n / 2] / 1000, (unsigned long long)ret[n * 95 / 100] / 1000,
           (unsigned long long)ret[n * 99 / 100] / 1000, (unsigned long long)ret[n - 1] / 1000, s10, s100);
    free(ret);
}

int main(int argc, char **argv)
{
    if (argc > 1 && !strcmp(argv[1], "rien")) return 0;
    setvbuf(stdout, NULL, _IOLBF, 0);
    int secondes = argc > 1 ? atoi(argv[1]) : 3;
    if (secondes < 1) secondes = 1;
    /* Le mesureur est interactif ; ses perturbateurs (fils forkes) le
     * redeviennent normaux. */
    const char *phases[] = { "repos", "calcul", "fork", "munmap", "journal", "spawn" };
    const char *seule = argc > 2 ? argv[2] : 0;
    int gentillesse = argc > 3 ? atoi(argv[3]) : -5;
    for (unsigned i = 0; i < sizeof phases / sizeof *phases; i++) {
        if (seule && strcmp(seule, "toutes") && strcmp(seule, phases[i])) continue;
        pid_t m = fork();
        if (m == 0) {
            setpriority(PRIO_PROCESS, 0, gentillesse);
            mesure(phases[i], secondes, argv[0]);
            _exit(0);
        }
        int etat;
        if (m < 0 || waitpid(m, &etat, 0) != m || !WIFEXITED(etat) || WEXITSTATUS(etat))
            return 1;
    }
    printf("LATENCE_PROBE_FIN\n");
    return 0;
}
