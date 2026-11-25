/*
 * VFORK-PROBE -- BOUCHAUD_VFORK_REEL_V1 et BOUCHAUD_CLONE_CHILD_SETTID_V1.
 *
 * Ce qui est verifie, dans le vrai noyau, avec la glibc :
 *
 *   1. `vfork()` (appel 58) PARTAGE la memoire : une ecriture du fils est vue
 *      par le pere, et le pere ne repart qu'apres la sortie du fils.
 *   2. `posix_spawn` (glibc : `clone(CLONE_VM|CLONE_VFORK)` sur une pile
 *      mmap) lance N fils qui font `execve`, pendant qu'un AUTRE fil du pere
 *      ecrit sans arret dans un tampon. TEMOIN NEGATIF de la passation du
 *      17/11 : l'`execve` d'un fils vfork ne doit pas detruire l'espace du
 *      pere. A la fin, le tampon du pere est intact et son fil vit encore.
 *   3. Un fils vfork dont l'`execve` echoue sort par `_exit(127)` : le pere
 *      reprend et lit 127.
 *   4. Apres `fork()`, `pthread_kill(fil principal)` depuis un second fil du
 *      fils atteint le fils (CLONE_CHILD_SETTID ecrit le tid dans l'espace du
 *      fils), et `getppid()` rend le pere vivant.
 *   5. Mesure A/B : cout moyen et pire d'un `fork`+`execve` et d'un
 *      `posix_spawn`, pere avec quelques Mio residents.
 *
 *     VFORK_PROBE_OK ...      tout est vrai
 *     VFORK_PROBE_FAIL raison=...
 *
 * Usage : vfork-probe [N]       (N lancements par mesure, 20 par defaut)
 *         vfork-probe N tid     (l'epreuve 4 seule : temoin sur un noyau
 *                                 qui n'aurait pas le partage vfork)
 *         vfork-probe --enfant  (cible de l'execve : sort 0 aussitot)
 */
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <spawn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

static long maintenant_us(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000L + t.tv_nsec / 1000L;
}

static int echec(const char *raison, long valeur)
{
    printf("VFORK_PROBE_FAIL raison=%s valeur=%ld\n", raison, valeur);
    fflush(stdout);
    return 1;
}

/* --- 1. vfork partage la memoire ---------------------------------------- */

static volatile int partage_vu;

static int epreuve_vfork_partage(void)
{
    partage_vu = 0;
    pid_t pid = vfork();
    if (pid < 0)
        return echec("vfork", errno);
    if (pid == 0) {
        partage_vu = 4242;
        _exit(7);
    }
    /* Le pere ne repart qu'apres la sortie du fils : l'ecriture est la. */
    if (partage_vu != 4242)
        return echec("vfork_memoire_non_partagee", partage_vu);
    int statut = 0;
    if (waitpid(pid, &statut, 0) != pid)
        return echec("vfork_waitpid", errno);
    if (!WIFEXITED(statut) || WEXITSTATUS(statut) != 7)
        return echec("vfork_statut", statut);
    return 0;
}

/* --- 2. posix_spawn sous ecriture concurrente du pere ------------------- */

#define TAMPON_MOTS (256 * 1024) /* 1 Mio */
static uint32_t *tampon;
static volatile int arret_ecrivain;
static volatile uint64_t tours_ecrivain;

static void *ecrivain(void *arg)
{
    (void)arg;
    uint32_t tour = 0;
    while (!arret_ecrivain) {
        tour++;
        for (size_t i = 0; i < TAMPON_MOTS; i += 1024)
            tampon[i] = tour ^ (uint32_t)i;
        /* Relecture : un espace detruit ou remplace se voit ici. */
        for (size_t i = 0; i < TAMPON_MOTS; i += 1024)
            if (tampon[i] != (tour ^ (uint32_t)i)) {
                tours_ecrivain = (uint64_t)-1;
                return NULL;
            }
        tours_ecrivain++;
    }
    return NULL;
}

static int lance_spawn(const char *moi, long *duree_us)
{
    char *argv[] = { (char *)moi, (char *)"--enfant", NULL };
    pid_t pid;
    long t0 = maintenant_us();
    int r = posix_spawn(&pid, moi, NULL, NULL, argv, environ);
    *duree_us = maintenant_us() - t0;
    if (r != 0)
        return r;
    int statut = 0;
    if (waitpid(pid, &statut, 0) != pid)
        return 1000 + errno;
    if (!WIFEXITED(statut) || WEXITSTATUS(statut) != 0)
        return 2000 + statut;
    return 0;
}

static int lance_fork_exec(const char *moi, long *duree_us)
{
    long t0 = maintenant_us();
    pid_t pid = fork();
    if (pid < 0)
        return errno;
    if (pid == 0) {
        char *argv[] = { (char *)moi, (char *)"--enfant", NULL };
        execve(moi, argv, environ);
        _exit(127);
    }
    *duree_us = maintenant_us() - t0;
    int statut = 0;
    if (waitpid(pid, &statut, 0) != pid)
        return 1000 + errno;
    if (!WIFEXITED(statut) || WEXITSTATUS(statut) != 0)
        return 2000 + statut;
    return 0;
}

/* --- 3. execve rate dans un fils vfork ---------------------------------- */

static int epreuve_execve_rate(void)
{
    char *argv[] = { (char *)"/inexistant/nulle-part", NULL };
    pid_t pid;
    int r = posix_spawn(&pid, argv[0], NULL, NULL, argv, environ);
    if (r == 0) {
        /* Une glibc peut aussi rendre le pid, le fils sortant en 127. */
        int statut = 0;
        if (waitpid(pid, &statut, 0) != pid)
            return echec("rate_waitpid", errno);
        if (!WIFEXITED(statut) || WEXITSTATUS(statut) != 127)
            return echec("rate_statut", statut);
        return 0;
    }
    /* La glibc rapporte l'erreur de l'execve du fils (ENOENT) : il faut
     * donc que le pere ait bien attendu le fils, sinon il aurait lu 0. */
    if (r != ENOENT)
        return echec("rate_errno", r);
    return 0;
}

/* --- 4. Apres fork, le tid range par la glibc est celui du fils ---------
 *
 * `pthread_kill(fil_principal)` depuis un AUTRE fil lit `pd->tid`, que le
 * noyau a du ecrire par CLONE_CHILD_SETTID. Avec le tid copie du pere, le
 * signal part vers un fil du pere (ou nulle part) et le gestionnaire du fils
 * ne tourne jamais. `raise()` ne le verrait pas : la glibc y refait gettid.
 */

static volatile sig_atomic_t recu;
static pthread_t principal;
static void gestionnaire(int s) { (void)s; recu = 1; }

static void *tireur(void *arg)
{
    (void)arg;
    return (void *)(intptr_t)pthread_kill(principal, SIGUSR1);
}

static int epreuve_tid_apres_fork(void)
{
    pid_t pere = getpid();
    pid_t pid = fork();
    if (pid < 0)
        return echec("fork", errno);
    if (pid == 0) {
        signal(SIGUSR1, gestionnaire);
        principal = pthread_self();
        pthread_t t;
        void *r = (void *)1;
        if (pthread_create(&t, NULL, tireur, NULL) != 0)
            _exit(5);
        pthread_join(t, &r);
        for (int i = 0; i < 1000 && !recu; i++)
            usleep(1000);
        if (r != NULL)
            _exit(3);
        if (!recu)
            _exit(4);
        /* BOUCHAUD_GETPPID_V1 : le pere vivant, pas 1. */
        _exit(getppid() == pere ? 0 : 6);
    }
    int statut = 0;
    if (waitpid(pid, &statut, 0) != pid)
        return echec("tid_waitpid", errno);
    if (!WIFEXITED(statut) || WEXITSTATUS(statut) != 0)
        return echec("tid_apres_fork", WIFEXITED(statut) ? WEXITSTATUS(statut) : -statut);
    return 0;
}

int main(int argc, char **argv)
{
    if (argc > 1 && strcmp(argv[1], "--enfant") == 0)
        return 0;
    long n = argc > 1 ? atol(argv[1]) : 20;
    if (n <= 0)
        n = 20;
    /* Chemin absolu de ce binaire : argv[0] s'il l'est (essai sur l'hote),
     * sinon l'emplacement que lui donne le scenario. */
    const char *moi = argv[0][0] == '/' ? argv[0] : "/bin/vfork-probe";

    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("VFORK_PROBE_BEGIN n=%ld\n", n);
    if (argc > 2 && strcmp(argv[2], "tid") == 0) {
        if (epreuve_tid_apres_fork())
            return 1;
        printf("VFORK_PROBE_ETAPE tid_apres_fork=ok\n");
        printf("VFORK_PROBE_OK seule=tid\n");
        return 0;
    }

    if (epreuve_vfork_partage())
        return 1;
    printf("VFORK_PROBE_ETAPE vfork_partage=ok\n");
    if (epreuve_execve_rate())
        return 1;
    printf("VFORK_PROBE_ETAPE execve_rate=ok\n");
    if (epreuve_tid_apres_fork())
        return 1;
    printf("VFORK_PROBE_ETAPE tid_apres_fork=ok\n");

    /* Un pere qui pese : 1 Mio d'ecrivain + 7 Mio residents. */
    tampon = malloc(TAMPON_MOTS * sizeof(uint32_t));
    size_t lest_taille = 7u << 20;
    unsigned char *lest = malloc(lest_taille);
    if (!tampon || !lest)
        return echec("malloc", 0);
    memset(tampon, 0, TAMPON_MOTS * sizeof(uint32_t));
    for (size_t i = 0; i < lest_taille; i++)
        lest[i] = (unsigned char)(i * 31u);

    pthread_t fil;
    if (pthread_create(&fil, NULL, ecrivain, NULL) != 0)
        return echec("pthread_create", 0);

    long spawn_total = 0, spawn_pire = 0, fork_total = 0, fork_pire = 0;
    for (long i = 0; i < n; i++) {
        long d = 0;
        int r = lance_spawn(moi, &d);
        if (r)
            return echec("posix_spawn", r);
        spawn_total += d;
        if (d > spawn_pire)
            spawn_pire = d;
        r = lance_fork_exec(moi, &d);
        if (r)
            return echec("fork_exec", r);
        fork_total += d;
        if (d > fork_pire)
            fork_pire = d;
    }
    arret_ecrivain = 1;
    pthread_join(fil, NULL);
    if (tours_ecrivain == (uint64_t)-1)
        return echec("tampon_du_pere_corrompu", 0);
    for (size_t i = 0; i < lest_taille; i++)
        if (lest[i] != (unsigned char)(i * 31u))
            return echec("lest_du_pere_corrompu", (long)i);

    printf("VFORK_PROBE_MESURE n=%ld spawn_moy_us=%ld spawn_pire_us=%ld "
           "fork_moy_us=%ld fork_pire_us=%ld tours_ecrivain=%llu\n",
           n, spawn_total / n, spawn_pire, fork_total / n, fork_pire,
           (unsigned long long)tours_ecrivain);
    printf("VFORK_PROBE_OK\n");
    return 0;
}
