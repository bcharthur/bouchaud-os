/*
 * ORPHELIN-PROBE -- BOUCHAUD_ORPHELINS_RECOLTES_V1 et BOUCHAUD_GETPPID_V1.
 *
 * Il n'y a pas d'init pour adopter les orphelins : le noyau doit les
 * recolter lui-meme, sans quoi ils restent zombies pour toujours.
 *
 *   P --fork--> C --fork--> G1 (sort tout de suite : zombie de C)
 *                 \--fork--> G2 (dort 300 ms, puis ecrit getppid() et sort)
 *   C sort SANS attendre ses fils ; P recolte C.
 *
 * Exige :
 *   1. G1 -- zombie au moment de la mort de C -- disparait (kill(G1,0) rend
 *      ESRCH) : regle « fils deja zombies » ;
 *   2. G2 voit getppid() == 1 apres la mort de C, puis, mort a son tour,
 *      disparait aussi : regle « pere disparu ».
 *
 *     ORPHELIN_PROBE_OK          ORPHELIN_PROBE_FAIL raison=...
 */
#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static long ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000L + t.tv_nsec / 1000000L;
}

static int echec(const char *raison, long v)
{
    printf("ORPHELIN_PROBE_FAIL raison=%s valeur=%ld\n", raison, v);
    return 1;
}

/* Le pid a-t-il disparu de la table (ESRCH) dans le delai ? */
static int disparait(pid_t pid, long delai_ms)
{
    long t0 = ms();
    while (ms() - t0 < delai_ms) {
        if (kill(pid, 0) != 0 && errno == ESRCH)
            return 1;
        usleep(10000);
    }
    return 0;
}

int main(void)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("ORPHELIN_PROBE_BEGIN\n");
    int pids[2], ppid_g2[2];
    if (pipe(pids) != 0 || pipe(ppid_g2) != 0)
        return echec("pipe", errno);

    pid_t c = fork();
    if (c < 0)
        return echec("fork_c", errno);
    if (c == 0) {
        pid_t g1 = fork();
        if (g1 == 0)
            _exit(0);
        pid_t g2 = fork();
        if (g2 == 0) {
            usleep(300000);
            int pp = (int)getppid();
            if (write(ppid_g2[1], &pp, sizeof pp) != sizeof pp)
                _exit(2);
            _exit(0);
        }
        /* Laisser G1 devenir zombie, puis mourir sans attendre personne. */
        usleep(100000);
        pid_t paire[2] = { g1, g2 };
        if (write(pids[1], paire, sizeof paire) != sizeof paire)
            _exit(3);
        _exit(0);
    }
    close(pids[1]);
    close(ppid_g2[1]);
    pid_t paire[2];
    if (read(pids[0], paire, sizeof paire) != sizeof paire)
        return echec("pids", errno);
    int statut = 0;
    if (waitpid(c, &statut, 0) != c || !WIFEXITED(statut) || WEXITSTATUS(statut) != 0)
        return echec("statut_c", statut);

    if (!disparait(paire[0], 2000))
        return echec("g1_zombie_pour_toujours", paire[0]);
    printf("ORPHELIN_PROBE_ETAPE fils_zombie_recolte=ok pid=%d\n", (int)paire[0]);

    int pp = -1;
    if (read(ppid_g2[0], &pp, sizeof pp) != sizeof pp)
        return echec("g2_muet", errno);
    if (pp != 1)
        return echec("g2_getppid", pp);
    if (!disparait(paire[1], 2000))
        return echec("g2_zombie_pour_toujours", paire[1]);
    printf("ORPHELIN_PROBE_ETAPE pere_disparu_recolte=ok pid=%d getppid=%d\n", (int)paire[1], pp);
    printf("ORPHELIN_PROBE_OK\n");
    return 0;
}
