/*
 * BOUCHAUD_FUTEX_REVEIL_COMPTE_V1 -- reproducteur deterministe de #376.
 *
 *   futex-reveil-echeance [tours]
 *
 * Defaut : un attendant TEMPORISE choisi par FUTEX_WAKE, mais qui ne reprend
 * le processeur qu'apres son echeance, rendait -ETIMEDOUT -- alors que le
 * reveilleur l'avait COMPTE comme reveille (FUTEX_WAKE rendait 1). Sous
 * Linux c'est impossible : un attendant retire de la file par un reveilleur
 * rend 0 (futex_unqueue echoue), quelle que soit l'heure de sa reprise.
 * Le reveil est alors consomme sans etre livre : la condvar de la glibc
 * envoie l'attendant dans __condvar_cancel_waiting sans prendre le signal, et
 * l'autre attendant n'est jamais reveille (gel d'ImageDecoder, #376).
 *
 * Tous les fils sont epingles sur le CPU 0 : le reveilleur reveille l'attendant
 * juste avant son echeance puis garde le processeur au-dela -- l'attendant
 * reprend forcement en retard.
 *
 * Niveau 1 (ABI) : deux attendants sur le meme mot -- X temporise 20 ms, W
 *   sans borne. Le mot change, puis FUTEX_WAKE(1) de 19,0 a 20,9 ms apres
 *   l'entree de X dans l'attente (pas de 0,1 ms) ; le reveilleur tourne ensuite
 *   5 ms. Si X rend ETIMEDOUT, il n'a pas pris le reveil : W, gare, doit etre
 *   reveille. Reveil perdu = W toujours gare 1 s plus tard.
 * Niveau 2 (glibc) : deux attendants sur une condvar -- X en timedwait 20 ms,
 *   W en wait sans borne ; UN pthread_cond_signal de -1,0 a +0,9 ms autour de
 *   l'echeance de X, puis 5 ms de calcul. Si X rend ETIMEDOUT, le signal appartient a W : W doit se reveiller.
 *   Reveil perdu = W toujours bloque 2 s plus tard.
 *
 * Sortie : FUTEX_REVEIL niveau=1 tours=N ... reveils_perdus=V
 *          FUTEX_REVEIL niveau=2 tours=N x_expire=E pertes=P
 *          FUTEX_REVEIL_OK (V = 0 et P = 0) ou FUTEX_REVEIL_KO ; un gel du
 *          niveau 2 imprime FUTEX_REVEIL_GEL (etat de la condvar).
 */
#define _GNU_SOURCE
#include <errno.h>
#include <linux/futex.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

static uint64_t ns_mono(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (uint64_t)t.tv_sec * 1000000000ull + (uint64_t)t.tv_nsec;
}

static void tourne_ms(unsigned ms)
{
    uint64_t fin = ns_mono() + (uint64_t)ms * 1000000ull;
    while (ns_mono() < fin)
        ;
}

static void dors_ms(unsigned ms)
{
    struct timespec t = { 0, (long)ms * 1000000L };
    nanosleep(&t, 0);
}

static void epingle_cpu0(void)
{
    cpu_set_t c;
    CPU_ZERO(&c);
    CPU_SET(0, &c);
    sched_setaffinity(0, sizeof c, &c);
}

/* --- Niveau 1 : l'ABI futex brute ---------------------------------------- */

static _Atomic int mot;
static _Atomic int tour_x, fini_x, tour_w, fini_w, avant_w;
static _Atomic long resultat_x;
static _Atomic uint64_t debut_x;

/* X : attente TEMPORISEE (20 ms). Cree le premier : le reveil le trouve le
 * premier dans le registre des taches. */
static void *attendant_x(void *arg)
{
    (void)arg;
    epingle_cpu0();
    for (int t = 1;; t++) {
        while (atomic_load(&tour_x) < t)
            sched_yield();
        if (atomic_load(&tour_x) < 0)
            return 0;
        struct timespec delai = { 0, 20 * 1000000L };
        atomic_store(&debut_x, ns_mono());
        long r = syscall(SYS_futex, &mot, FUTEX_WAIT_PRIVATE, 0, &delai, 0, 0);
        atomic_store(&resultat_x, r == 0 ? 0 : -errno);
        atomic_store(&fini_x, t);
    }
}

/* W : attente SANS borne sur le meme mot. */
static void *attendant_w(void *arg)
{
    (void)arg;
    epingle_cpu0();
    for (int t = 1;; t++) {
        while (atomic_load(&tour_w) < t)
            sched_yield();
        if (atomic_load(&tour_w) < 0)
            return 0;
        atomic_store(&avant_w, t);
        while (atomic_load(&mot) == 0) {
            if (syscall(SYS_futex, &mot, FUTEX_WAIT_PRIVATE, 0, 0, 0, 0) == 0)
                break;
        }
        atomic_store(&fini_w, t);
    }
}

static int niveau1(int tours)
{
    pthread_t x, w;
    pthread_create(&x, 0, attendant_x, 0);
    pthread_create(&w, 0, attendant_w, 0);
    int perdus = 0, x_expire = 0, x_reveille = 0, imprecis = 0;
    for (int t = 1; t <= tours; t++) {
        atomic_store(&mot, 0);
        atomic_store(&tour_w, t);
        while (atomic_load(&avant_w) < t)
            sched_yield();
        dors_ms(2); /* W se gare */
        atomic_store(&debut_x, 0);
        atomic_store(&tour_x, t);
        while (atomic_load(&debut_x) == 0)
            sched_yield();
        /* Reveil de 19,0 a 20,9 ms apres l'entree de X dans l'attente
         * (echeance : 20 ms) : une partie des tours tombe entre l'echeance et
         * le traitement de X par le minuteur. */
        uint64_t cible = atomic_load(&debut_x) + 19000000ull + (uint64_t)((t - 1) % 20) * 100000ull;
        struct timespec c = { (time_t)(cible / 1000000000ull), (long)(cible % 1000000000ull) };
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &c, 0);
        atomic_store(&mot, 1);
        long n = syscall(SYS_futex, &mot, FUTEX_WAKE_PRIVATE, 1, 0, 0, 0);
        tourne_ms(5);
        while (atomic_load(&fini_x) < t)
            sched_yield();
        long r = atomic_load(&resultat_x);
        if (r == -ETIMEDOUT) {
            x_expire++;
            /* X n'a pas pris le reveil : il revenait a W, toujours gare. */
            uint64_t limite = ns_mono() + 1000000000ull;
            while (atomic_load(&fini_w) < t && ns_mono() < limite)
                dors_ms(1);
            if (atomic_load(&fini_w) < t) {
                perdus++;
                if (n == 1)
                    imprecis++; /* compte pour X, livre a personne */
            }
        } else {
            x_reveille++;
        }
        while (atomic_load(&fini_w) < t) {
            syscall(SYS_futex, &mot, FUTEX_WAKE_PRIVATE, 1, 0, 0, 0);
            dors_ms(1);
        }
    }
    atomic_store(&tour_x, -1);
    atomic_store(&tour_w, -1);
    printf("FUTEX_REVEIL niveau=1 tours=%d x_reveille=%d x_expire=%d reveils_perdus=%d dont_comptes_pour_x=%d\n",
        tours, x_reveille, x_expire, perdus, imprecis);
    return perdus;
}

/* --- Niveau 2 : la condvar de la glibc ----------------------------------- */

static pthread_mutex_t verrou = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t cond;
static _Atomic int x_resultat, w_reveille, x_fini, w_pret, x_pret;
static _Atomic uint64_t x_echeance;
static _Atomic int progres_n2, tour_n2;

/* Chien de garde du niveau 2 : aucun progres en 5 s = gel. Il imprime les
 * mots de la condvar (glibc >= 2.25 : __wseq, __g1_start, __g_refs[2],
 * __g_size[2], __g1_orig_size, __wrefs, __g_signals[2]). */
static void *garde_n2(void *arg)
{
    (void)arg;
    int vu = -1;
    uint64_t depuis = ns_mono();
    for (;;) {
        dors_ms(500);
        int p = atomic_load(&progres_n2);
        if (p != vu) {
            vu = p;
            depuis = ns_mono();
            continue;
        }
        if (ns_mono() - depuis > 5000000000ull) {
            unsigned *m = (unsigned *)&cond;
            printf("FUTEX_REVEIL_GEL niveau=2 tour=%d etape=%d x_resultat=%d x_fini=%d w_reveille=%d "
                   "g_refs=%u,%u g_size=%u,%u g1_orig_size=%#x wrefs=%#x g_signals=%#x,%#x wseq=%#x g1_start=%#x\n",
                atomic_load(&tour_n2), p & 7, atomic_load(&x_resultat), atomic_load(&x_fini),
                atomic_load(&w_reveille), m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11], m[0], m[2]);
            printf("FUTEX_REVEIL_KO gel_niveau2\n");
            _exit(4);
        }
    }
    return 0;
}

static void *x_temporise(void *arg)
{
    (void)arg;
    epingle_cpu0();
    struct timespec e;
    clock_gettime(CLOCK_MONOTONIC, &e);
    e.tv_nsec += 20 * 1000000L;
    if (e.tv_nsec >= 1000000000L) {
        e.tv_sec++;
        e.tv_nsec -= 1000000000L;
    }
    pthread_mutex_lock(&verrou);
    atomic_store(&x_echeance, (uint64_t)e.tv_sec * 1000000000ull + (uint64_t)e.tv_nsec);
    atomic_store(&x_pret, 1);
    int r = pthread_cond_timedwait(&cond, &verrou, &e);
    pthread_mutex_unlock(&verrou);
    atomic_store(&x_resultat, r);
    atomic_store(&x_fini, 1);
    return 0;
}

static void *w_sans_borne(void *arg)
{
    (void)arg;
    epingle_cpu0();
    pthread_mutex_lock(&verrou);
    atomic_store(&w_pret, 1);
    pthread_cond_wait(&cond, &verrou);
    pthread_mutex_unlock(&verrou);
    atomic_store(&w_reveille, 1);
    return 0;
}

static int niveau2(int tours)
{
    int pertes = 0, x_expire = 0;
    pthread_t garde;
    pthread_create(&garde, 0, garde_n2, 0);
    for (int t = 0; t < tours; t++) {
        atomic_store(&tour_n2, t);
        atomic_fetch_add(&progres_n2, 8);
        pthread_condattr_t a;
        pthread_condattr_init(&a);
        pthread_condattr_setclock(&a, CLOCK_MONOTONIC);
        pthread_cond_init(&cond, &a);
        atomic_store(&x_resultat, -1);
        atomic_store(&w_reveille, 0);
        atomic_store(&x_fini, 0);
        atomic_store(&w_pret, 0);
        atomic_store(&x_pret, 0);
        pthread_t w, x;
        /* X d'abord : le reveil le trouve le premier. */
        pthread_create(&x, 0, x_temporise, 0);
        while (!atomic_load(&x_pret))
            sched_yield();
        pthread_create(&w, 0, w_sans_borne, 0);
        while (!atomic_load(&w_pret))
            sched_yield();
        /* Signal de -1,0 a +0,9 ms autour de l'echeance de X. */
        uint64_t cible = atomic_load(&x_echeance) - 1000000ull + (uint64_t)(t % 20) * 100000ull;
        struct timespec c = { (time_t)(cible / 1000000000ull), (long)(cible % 1000000000ull) };
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &c, 0);
        pthread_cond_signal(&cond);
        atomic_fetch_add(&progres_n2, 1);
        tourne_ms(5);
        while (!atomic_load(&x_fini))
            sched_yield();
        atomic_fetch_add(&progres_n2, 1);
        int r = atomic_load(&x_resultat);
        if (r == ETIMEDOUT) {
            x_expire++;
            /* Le signal etait pour W : il a 2 s pour se reveiller. */
            uint64_t limite = ns_mono() + 2000000000ull;
            while (!atomic_load(&w_reveille) && ns_mono() < limite)
                dors_ms(1);
            if (!atomic_load(&w_reveille))
                pertes++;
        }
        /* Liberer W s'il dort encore (signal legitime suivant). */
        while (!atomic_load(&w_reveille)) {
            pthread_mutex_lock(&verrou);
            pthread_cond_broadcast(&cond);
            pthread_mutex_unlock(&verrou);
            dors_ms(1);
        }
        atomic_fetch_add(&progres_n2, 1);
        pthread_join(x, 0);
        pthread_join(w, 0);
        pthread_cond_destroy(&cond);
    }
    printf("FUTEX_REVEIL niveau=2 tours=%d x_expire=%d pertes=%d\n", tours, x_expire, pertes);
    return pertes;
}

int main(int argc, char **argv)
{
    int tours = argc > 1 ? atoi(argv[1]) : 40;
    setvbuf(stdout, 0, _IOLBF, 0);
    epingle_cpu0();
    int v = niveau1(tours);
    int p = niveau2(tours);
    printf(v == 0 && p == 0 ? "FUTEX_REVEIL_OK\n" : "FUTEX_REVEIL_KO reveils_perdus=%d pertes=%d\n", v, p);
    return v == 0 && p == 0 ? 0 : 1;
}
