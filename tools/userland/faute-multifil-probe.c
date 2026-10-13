/*
 * Sonde : une exception CPU en ring 3 dans un processus MULTI-FILS tue le
 * processus, jamais le noyau.
 *
 * BOUCHAUD_FAUTE_MULTIFIL_V1 (preuve de BOUCHAUD_P19_FATAL_USER_IRQ_EXIT_V1)
 *
 * Les gestionnaires `extern "x86-interrupt"` du noyau commencent avec IF=0.
 * Apres #UD, #GP ou #DE venus du ring 3, `kill_faulting_task` ne revient pas
 * par iretq : il appelle `exit_group`, qui marque les freres et ATTEND leur
 * extinction (`attend_extinction_freres` -> `sleep_ticks_noyau` ->
 * `schedule`). L'ordonnanceur exige IF=1 : sans la reouverture P19, un
 * processus seul mourait proprement (rien a attendre), mais un processus
 * multi-fils -- WebContent, tout navigateur -- emportait le noyau.
 * `faute-noncanonique-probe` ne voyait rien : ses fils fautifs n'ont qu'un
 * fil.
 *
 * Ici chaque fils fautif a trois freres, chacun dans un etat different au
 * moment de la faute : endormi (nanosleep), en calcul pur (aucun appel
 * systeme), bloque dans read() sur un tube que personne n'ecrit. La faute
 * part soit du fil principal, soit d'un frere. Quatre exceptions :
 *
 *   ud2     #UD  (la forme d'un VERIFY d'AK : ak_trap)
 *   div     #DE  (division entiere par zero)
 *   cli     #GP  (instruction privilegiee en ring 3)
 *   nul     #PF  (ecriture a l'adresse 0 ; deja IF=1 dans le noyau)
 *
 * Le pere attend chaque fils au plus 30 s (sondage borne). Exige : chaque
 * fils meurt d'un signal, aucun ne survit, aucun ne sort normalement.
 *
 * Usage : faute-multifil-probe [tours]   (defaut 3)
 * Sortie : FAUTE_MULTIFIL_OK essais=N  ou  FAUTE_MULTIFIL_ECHEC n=...
 */
#define _GNU_SOURCE
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

enum { UD2, DIV, CLI, NUL, GENRES };
static const char *const NOMS[GENRES] = { "ud2", "div", "cli", "nul" };

static atomic_int prets;
static int tube[2];
static int genre_choisi;

__attribute__((noinline)) static void faute(int genre)
{
    switch (genre) {
    case UD2:
        __asm__ volatile("ud2");
        break;
    case DIV:
        __asm__ volatile("xor %%ecx, %%ecx\n\tmov $1, %%eax\n\txor %%edx, %%edx\n\tdiv %%ecx"
                         ::: "eax", "ecx", "edx", "cc");
        break;
    case CLI:
        __asm__ volatile("cli");
        break;
    case NUL:
        *(volatile int *)(uintptr_t)0 = 1;
        break;
    }
}

static void *dormeur(void *arg)
{
    (void)arg;
    atomic_fetch_add(&prets, 1);
    for (;;) {
        struct timespec t = { 0, 20 * 1000 * 1000 };
        nanosleep(&t, NULL);
    }
    return NULL;
}

static void *calculateur(void *arg)
{
    (void)arg;
    atomic_fetch_add(&prets, 1);
    volatile uint64_t x = 0;
    for (;;)
        x++;
    return NULL;
}

static void *lecteur(void *arg)
{
    (void)arg;
    atomic_fetch_add(&prets, 1);
    char c;
    for (;;)
        (void)!read(tube[0], &c, 1);
    return NULL;
}

static void *fautif(void *arg)
{
    (void)arg;
    while (atomic_load(&prets) < 3)
        sched_yield();
    faute(genre_choisi);
    return NULL;
}

/* Le fils : trois freres, puis la faute depuis le fil principal ou un 4e fil. */
static void fils(int genre, int depuis_frere)
{
    genre_choisi = genre;
    if (pipe(tube) != 0)
        _exit(90);
    pthread_t t[4];
    if (pthread_create(&t[0], NULL, dormeur, NULL) != 0
        || pthread_create(&t[1], NULL, calculateur, NULL) != 0
        || pthread_create(&t[2], NULL, lecteur, NULL) != 0)
        _exit(91);
    if (depuis_frere) {
        if (pthread_create(&t[3], NULL, fautif, NULL) != 0)
            _exit(92);
        for (;;)
            pause();
    }
    fautif(NULL);
    _exit(93); /* la faute n'a pas eu lieu */
}

static int64_t ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (int64_t)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    int tours = argc > 1 ? atoi(argv[1]) : 3;
    if (tours < 1)
        tours = 1;
    int echecs = 0, essais = 0;
    for (int tour = 0; tour < tours; tour++) {
        for (int genre = 0; genre < GENRES; genre++) {
            for (int depuis_frere = 0; depuis_frere < 2; depuis_frere++) {
                essais++;
                pid_t p = fork();
                if (p < 0) {
                    printf("  %-4s %-9s fork ECHEC\n", NOMS[genre], depuis_frere ? "frere" : "principal");
                    echecs++;
                    continue;
                }
                if (p == 0)
                    fils(genre, depuis_frere);
                int st = 0;
                pid_t r = 0;
                int64_t debut = ms();
                while ((r = waitpid(p, &st, WNOHANG)) == 0 && ms() - debut < 30000) {
                    struct timespec t = { 0, 10 * 1000 * 1000 };
                    nanosleep(&t, NULL);
                }
                int ok = r == p && WIFSIGNALED(st);
                if (r == 0) {
                    kill(p, SIGKILL);
                    waitpid(p, &st, 0);
                }
                char quoi[64];
                if (r == 0)
                    snprintf(quoi, sizeof quoi, "ne meurt pas en 30 s");
                else if (WIFSIGNALED(st))
                    snprintf(quoi, sizeof quoi, "signal %d", WTERMSIG(st));
                else
                    snprintf(quoi, sizeof quoi, "sortie %d", WIFEXITED(st) ? WEXITSTATUS(st) : -1);
                printf("  tour=%d %-4s %-9s %-22s %lld ms %s\n", tour, NOMS[genre],
                       depuis_frere ? "frere" : "principal", quoi,
                       (long long)(ms() - debut), ok ? "ok" : "ECHEC");
                echecs += !ok;
            }
        }
    }
    if (echecs == 0)
        printf("FAUTE_MULTIFIL_OK essais=%d\n", essais);
    else
        printf("FAUTE_MULTIFIL_ECHEC n=%d essais=%d\n", echecs, essais);
    return echecs != 0;
}
