/*
 * Sonde : la comptabilite CPU reste coherente sous echantillonnage concurrent.
 *
 * BOUCHAUD_COMPTA_STRESS_V1
 *
 * Run 37589903681 (KVM) : panique noyau « task: runtime > fenetre tid=118
 * delta=35786689 window=33031990 ». Le lecteur (`mesure_processus`) lisait
 * le curseur de la tranche en cours et les compteurs cumules d'une tache a
 * deux instants differents, pendant que le coeur de la tache les mettait a
 * jour : la tranche comptait deux fois. Correctif : sequence par tache
 * (BOUCHAUD_COMPTA_SEQLOCK_V1).
 *
 * Cette sonde fabrique la course a volonte :
 *   - 2 x ncpu fils de calcul qui changent de contexte sans arret (calcul
 *     court, sched_yield, nanosleep) ; plus de fils que de coeurs : le
 *     repartiteur les deplace d'un coeur a l'autre. Ils demandent aussi une
 *     affinite tournante (sched_setaffinity) ; Bouchaud l'accepte sans
 *     effet aujourd'hui -- le compteur s'appelle donc `setaffinity`, pas
 *     `migrations` ;
 *   - 2 fils lecteurs qui relisent /proc/self/stat (lecteur par processus) et
 *     /proc/stat (lecteur de toutes les taches) aussi vite que possible ;
 *   - 1 fil qui cree des processus fils multi-fils ephemeres, lit leur
 *     /proc/<pid>/stat vivants puis zombies, et les recolte (pid et tid
 *     recycles par le noyau) ;
 *   - pendant ce temps l'echantillonneur du noyau passe toutes les 5 s, avec
 *     son assertion (noyau debug).
 *
 * Invariants verifies a chaque lecture (unites : tick de /proc, T = 1/HZ) :
 *   I1  utime, stime et (utime + stime) ne reculent jamais, par processus ;
 *       user et system de /proc/stat non plus.
 *   I2  (utime + stime) n'avance pas plus vite que le temps mur fois le
 *       nombre de CPU : entre deux lectures k-1 et k encadrees par les
 *       horloges a(k) <= lecture <= b(k),
 *           D <= ncpu * (b(k) - a(k-1)) / T + 3
 *       +2 : utime et stime sont tronques separement (floor(u/T) +
 *       floor(s/T) perd moins de 2 ticks, donc l'ecart de deux lectures
 *       gagne au plus 2) ; +1 : ecart d'horloge entre coeurs (le repli d'une
 *       tranche est date par le coeur de la tache, la tranche vive par celui
 *       du lecteur), borne par une milliseconde par coeur, soit moins d'un
 *       tick de 10 ms tant que ncpu < 10.
 *   I3  le noyau ne panique pas (verifie par le banc : pas de KERNEL PANIC).
 *
 * ncpu : compte sur le masque d'un pthread. Le fil racine d'un lancement
 * synchrone est epingle a son coeur (lifecycle.rs), sysconf() y voit 1 ; ses
 * pthreads naissent avec la machine entiere.
 *
 * Sortie : `COMPTA_STRESS lectures=... reculs=... depassements=...` puis
 * `COMPTA_STRESS_OK` ou `COMPTA_STRESS_ECHEC`.
 *
 *   compta-stress-probe [secondes]   (defaut 30)
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static atomic_int fin;
static int ncpu = 1;
static long tick_ns = 10000000;

static atomic_long lectures, reculs, depassements, setaffinite, commutations, fils_crees, lectures_fils, echecs_lecture;

static long long maintenant_ns(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000000000LL + t.tv_nsec;
}

/* Lit les champs 14 (utime) et 15 (stime) de /proc/<pid>/stat. */
/* Derniere ligne brute lue par ce fil : imprimee avec la precedente quand
 * une lecture recule (etat, nombre de fils, tous les champs). */
static __thread char ligne_brute[256];

static int lit_stat(const char *chemin, unsigned long long *u, unsigned long long *s)
{
    char tampon[1024];
    int fd = open(chemin, O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return -1;
    ssize_t n = read(fd, tampon, sizeof tampon - 1);
    close(fd);
    if (n <= 0)
        return -1;
    tampon[n] = 0;
    size_t k = (size_t)n < sizeof ligne_brute - 1 ? (size_t)n : sizeof ligne_brute - 1;
    memcpy(ligne_brute, tampon, k);
    ligne_brute[k] = 0;
    char *nl = strchr(ligne_brute, '\n');
    if (nl)
        *nl = 0;
    // Le nom (champ 2) peut contenir des espaces : repartir de la derniere ')'.
    char *p = strrchr(tampon, ')');
    if (!p)
        return -1;
    p++;
    // Champs 3 a 15 : on saute 11 champs (3..13), puis utime, stime.
    for (int champ = 3; champ <= 13; champ++) {
        while (*p == ' ')
            p++;
        while (*p && *p != ' ')
            p++;
    }
    if (sscanf(p, " %llu %llu", u, s) != 2)
        return -1;
    return 0;
}

static int lit_stat_global(unsigned long long *u, unsigned long long *s)
{
    char tampon[512];
    int fd = open("/proc/stat", O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return -1;
    ssize_t n = read(fd, tampon, sizeof tampon - 1);
    close(fd);
    if (n <= 0)
        return -1;
    tampon[n] = 0;
    unsigned long long nice = 0;
    if (sscanf(tampon, "cpu %llu %llu %llu", u, &nice, s) != 3)
        return -1;
    *u += nice;
    return 0;
}

/* Suit une serie de lectures d'un meme processus et verifie I1 et I2. */
struct suivi {
    int amorce;
    unsigned long long u, s;
    long long a; /* horloge juste avant la lecture precedente */
    char ligne[256]; /* ligne brute precedente (lectures de /proc/<pid>/stat) */
};

static void verifie(struct suivi *v, const char *quoi, long pid, unsigned long long u, unsigned long long s,
    long long a, long long b, int borne_cpu)
{
    atomic_fetch_add(&lectures, 1);
    if (v->amorce) {
        if (u < v->u || s < v->s) {
            if (atomic_fetch_add(&reculs, 1) < 8) {
                printf("COMPTA_RECUL %s pid=%ld utime=%llu<-%llu stime=%llu<-%llu ecart_ns=%lld\n", quoi, pid, u, v->u, s,
                    v->s, b - v->a);
                if (pid > 0)
                    printf("COMPTA_RECUL_AVANT %s\nCOMPTA_RECUL_APRES %s\n", v->ligne, ligne_brute);
            }
        }
        unsigned long long d = (u + s) - (v->u + v->s);
        long long mur = b - v->a;
        unsigned long long borne = (unsigned long long)((long double)borne_cpu * mur / tick_ns) + 3;
        if ((u + s) >= (v->u + v->s) && d > borne) {
            if (atomic_fetch_add(&depassements, 1) < 8)
                printf("COMPTA_DEPASSE %s pid=%ld delta_ticks=%llu borne=%llu mur_ns=%lld cpu=%d\n", quoi, pid, d, borne, mur,
                    borne_cpu);
        }
    }
    v->amorce = 1;
    v->u = u;
    v->s = s;
    v->a = a;
    if (pid > 0)
        memcpy(v->ligne, ligne_brute, sizeof v->ligne);
}

/* --- fils de calcul ------------------------------------------------------ */
static void *calcul(void *arg)
{
    long id = (long)arg;
    volatile unsigned long x = 0;
    unsigned tour = 0;
    while (!atomic_load(&fin)) {
        long long t0 = maintenant_ns();
        long long duree = 20000 + (long long)((id * 7919 + tour * 104729) % 400000);
        while (maintenant_ns() - t0 < duree)
            x += tour;
        switch (tour % 4) {
        case 0:
            sched_yield();
            break;
        case 1: {
            struct timespec d = { 0, 50000 };
            nanosleep(&d, NULL);
            break;
        }
        case 2:
            if (ncpu > 1) {
                cpu_set_t ens;
                CPU_ZERO(&ens);
                CPU_SET((int)((id + tour / 4) % ncpu), &ens);
                if (sched_setaffinity(0, sizeof ens, &ens) == 0)
                    atomic_fetch_add(&setaffinite, 1);
            }
            break;
        default:
            break;
        }
        atomic_fetch_add(&commutations, 1);
        tour++;
    }
    return NULL;
}

/* --- lecteurs ------------------------------------------------------------ */
static void *lecteur(void *arg)
{
    long id = (long)arg;
    struct suivi soi = { 0 }, global = { 0 };
    char chemin[64];
    snprintf(chemin, sizeof chemin, "/proc/%d/stat", (int)getpid());
    while (!atomic_load(&fin)) {
        unsigned long long u, s;
        long long a = maintenant_ns();
        int r = (id & 1) ? lit_stat_global(&u, &s) : lit_stat(chemin, &u, &s);
        long long b = maintenant_ns();
        if (r != 0) {
            atomic_fetch_add(&echecs_lecture, 1);
            continue;
        }
        if (id & 1)
            verifie(&global, "global", 0, u, s, a, b, ncpu);
        else
            verifie(&soi, "soi", getpid(), u, s, a, b, ncpu);
        // Alterner aussi l'autre source, pour croiser les deux lecteurs.
        a = maintenant_ns();
        r = (id & 1) ? lit_stat(chemin, &u, &s) : lit_stat_global(&u, &s);
        b = maintenant_ns();
        if (r == 0) {
            if (id & 1)
                verifie(&soi, "soi", getpid(), u, s, a, b, ncpu);
            else
                verifie(&global, "global", 0, u, s, a, b, ncpu);
        }
    }
    return NULL;
}

/* --- processus ephemeres : pid/tid recycles, lectures de zombies ---------- */
static void *fils_tourne(void *arg)
{
    (void)arg;
    volatile unsigned long x = 0;
    long long t0 = maintenant_ns();
    while (maintenant_ns() - t0 < 3000000)
        x++;
    return NULL;
}

static void *createur(void *arg)
{
    (void)arg;
    while (!atomic_load(&fin)) {
        pid_t p = fork();
        if (p == 0) {
            pthread_t t[3];
            for (int i = 0; i < 3; i++)
                pthread_create(&t[i], NULL, fils_tourne, NULL);
            fils_tourne(NULL);
            for (int i = 0; i < 3; i++)
                pthread_join(t[i], NULL);
            _exit(0);
        }
        if (p < 0) {
            usleep(10000);
            continue;
        }
        atomic_fetch_add(&fils_crees, 1);
        char chemin[64];
        snprintf(chemin, sizeof chemin, "/proc/%d/stat", (int)p);
        struct suivi v = { 0 };
        // Lire pendant la vie du fils, puis une fois devenu zombie.
        for (;;) {
            unsigned long long u, s;
            long long a = maintenant_ns();
            int r = lit_stat(chemin, &u, &s);
            long long b = maintenant_ns();
            if (r == 0) {
                // Un fils a 4 fils ne peut pas depasser min(4, ncpu) CPU.
                verifie(&v, "fils", p, u, s, a, b, ncpu < 4 ? ncpu : 4);
                atomic_fetch_add(&lectures_fils, 1);
            }
            int st;
            pid_t w = waitpid(p, &st, WNOHANG);
            if (w == p || w < 0)
                break;
        }
    }
    return NULL;
}

static void *compte_coeurs(void *arg)
{
    cpu_set_t ens;
    CPU_ZERO(&ens);
    if (sched_getaffinity(0, sizeof ens, &ens) == 0)
        *(int *)arg = CPU_COUNT(&ens);
    return NULL;
}

int main(int argc, char **argv)
{
    int secondes = argc > 1 ? atoi(argv[1]) : 30;
    if (secondes <= 0)
        secondes = 30;
    setvbuf(stdout, NULL, _IONBF, 0);
    long n = sysconf(_SC_NPROCESSORS_ONLN);
    ncpu = n > 0 ? (int)n : 1;
    int vu = 0;
    pthread_t sonde;
    if (pthread_create(&sonde, NULL, compte_coeurs, &vu) == 0) {
        pthread_join(sonde, NULL);
        if (vu > ncpu)
            ncpu = vu;
    }
    long hz = sysconf(_SC_CLK_TCK);
    if (hz > 0)
        tick_ns = 1000000000L / hz;
    int ncalcul = 2 * ncpu;
    printf("COMPTA_STRESS_DEBUT ncpu=%d fils_calcul=%d secondes=%d tick_ns=%ld\n", ncpu, ncalcul, secondes, tick_ns);

    pthread_t t[64];
    int nt = 0;
    for (long i = 0; i < ncalcul && nt < 60; i++)
        pthread_create(&t[nt++], NULL, calcul, (void *)i);
    for (long i = 0; i < 2; i++)
        pthread_create(&t[nt++], NULL, lecteur, (void *)i);
    pthread_create(&t[nt++], NULL, createur, NULL);

    sleep((unsigned)secondes);
    atomic_store(&fin, 1);
    for (int i = 0; i < nt; i++)
        pthread_join(t[i], NULL);
    while (waitpid(-1, NULL, WNOHANG) > 0) {
    }

    long l = atomic_load(&lectures), r = atomic_load(&reculs), d = atomic_load(&depassements);
    long f = atomic_load(&fils_crees), lf = atomic_load(&lectures_fils);
    printf("COMPTA_STRESS lectures=%ld lectures_fils=%ld reculs=%ld depassements=%ld setaffinity=%ld commutations=%ld "
           "fils=%ld echecs_lecture=%ld\n",
        l, lf, r, d, atomic_load(&setaffinite), atomic_load(&commutations), f, atomic_load(&echecs_lecture));
    // Une sonde qui n'a rien lu ne prouve rien : exiger du volume.
    int ok = r == 0 && d == 0 && l >= 1000 && f >= 10 && lf >= 10;
    printf(ok ? "COMPTA_STRESS_OK\n" : "COMPTA_STRESS_ECHEC\n");
    return ok ? 0 : 1;
}
