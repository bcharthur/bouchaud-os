/*
 * Sonde : la mort d'un fils MULTI-FILS (faute dans un de ses fils) doit
 * reveiller son pere par SIGCHLD, et waitpid(-1, WNOHANG) doit le rendre,
 * tue par SIGSEGV.
 *
 * BOUCHAUD_SIGCHLD_MULTIFIL_V1
 *
 * C'est EXACTEMENT le chemin par lequel Ladybird apprend qu'un de ses
 * services est mort (LibWebView/ProcessMonitor.cpp : SIGCHLD enregistre dans
 * la boucle d'evenements, puis waitpid(-1, WNOHANG)). Run 37584587000 : le
 * WebContent d'un onglet faute (PROCESS_FAULT pid=20 cr2=0x10), sa connexion
 * au Compositor se ferme, mais le navigateur ne voit JAMAIS la mort : ni
 * reprise, ni page d'erreur, ni [LB] PROCESS_EXIT, 65 s durant.
 *
 * Le pere imite la boucle de LibCore : un gestionnaire de SIGCHLD qui ecrit
 * dans un tube, un fil principal en poll() sur ce tube et sur une prise
 * partagee avec le fils, et un second fil bloque ailleurs (le signal peut
 * echoir a n'importe quel fil). Trois fils :
 *   mono       un seul fil, qui faute (temoin) ;
 *   principal  quatre fils (poll, nanosleep, attente de condition, read),
 *              le fil PRINCIPAL faute ;
 *   secondaire meme chose, un fil SECONDAIRE faute, le principal en poll.
 *
 * Pour chacun : delai de la fin de prise (EOF), delai du SIGCHLD, et ce que
 * waitpid rend. Sortie : une ligne `SIGCHLD_CAS cas=...`, puis
 * `SIGCHLD_MULTIFIL_OK` ou `SIGCHLD_MULTIFIL_ECHEC n=...`.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int tube_signal[2];

static void sur_sigchld(int sig)
{
    (void)sig;
    int e = errno;
    char c = 'c';
    (void)!write(tube_signal[1], &c, 1);
    errno = e;
}

static long maintenant_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

static void faute(void)
{
    volatile uintptr_t adresse = 0x10;
    *(volatile int *)adresse = 1;
}

/* --- fils du fils : chacun bloque a sa facon ------------------------------ */
static int tube_mort[2];
static pthread_mutex_t verrou = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t condition = PTHREAD_COND_INITIALIZER;

static void *en_poll(void *arg)
{
    (void)arg;
    struct pollfd p = { .fd = tube_mort[0], .events = POLLIN };
    for (;;)
        poll(&p, 1, -1);
    return NULL;
}

static void *en_sommeil(void *arg)
{
    (void)arg;
    for (;;) {
        struct timespec d = { .tv_sec = 1 };
        nanosleep(&d, NULL);
    }
    return NULL;
}

static void *en_condition(void *arg)
{
    (void)arg;
    pthread_mutex_lock(&verrou);
    for (;;)
        pthread_cond_wait(&condition, &verrou);
    return NULL;
}

static void *en_lecture(void *arg)
{
    (void)arg;
    char c;
    for (;;)
        (void)!read(tube_mort[0], &c, 1);
    return NULL;
}

static void *fautif(void *arg)
{
    (void)arg;
    usleep(400 * 1000);
    faute();
    return NULL;
}

static void fils(int cas)
{
    if (cas == 0) {
        usleep(400 * 1000);
        faute();
        _exit(100);
    }
    (void)!pipe(tube_mort);
    pthread_t t;
    pthread_create(&t, NULL, en_poll, NULL);
    pthread_create(&t, NULL, en_sommeil, NULL);
    pthread_create(&t, NULL, en_condition, NULL);
    pthread_create(&t, NULL, en_lecture, NULL);
    if (cas == 1) {
        usleep(400 * 1000);
        faute();
    } else {
        pthread_create(&t, NULL, fautif, NULL);
        struct pollfd p = { .fd = tube_mort[0], .events = POLLIN };
        for (;;)
            poll(&p, 1, -1);
    }
    _exit(101);
}

/* --- pere ----------------------------------------------------------------- */
static void *pere_ailleurs(void *arg)
{
    (void)arg;
    int t[2];
    (void)!pipe(t);
    struct pollfd p = { .fd = t[0], .events = POLLIN };
    for (;;)
        poll(&p, 1, -1);
    return NULL;
}

int main(void)
{
    static const char *const noms[] = { "mono", "principal", "secondaire" };
    setvbuf(stdout, NULL, _IONBF, 0);
    if (pipe2(tube_signal, O_CLOEXEC | O_NONBLOCK) != 0) {
        printf("SIGCHLD_MULTIFIL_ECHEC n=1 raison=pipe2\n");
        return 1;
    }
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = sur_sigchld;
    sa.sa_flags = SA_RESTART;
    sigaction(SIGCHLD, &sa, NULL);
    pthread_t autre;
    pthread_create(&autre, NULL, pere_ailleurs, NULL);

    int echecs = 0;
    for (int cas = 0; cas < 3; cas++) {
        int prise[2];
        socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, prise);
        char c;
        while (read(tube_signal[0], &c, 1) == 1) {
        }
        long t0 = maintenant_ms();
        pid_t p = fork();
        if (p == 0) {
            close(prise[0]);
            fils(cas);
        }
        close(prise[1]);
        long t_eof = -1, t_sig = -1;
        pid_t rendu = 0;
        int statut = 0;
        while (maintenant_ms() - t0 < 10000 && (t_eof < 0 || rendu <= 0)) {
            struct pollfd f[2] = { { .fd = tube_signal[0], .events = POLLIN }, { .fd = prise[0], .events = POLLIN } };
            int n = poll(f, t_eof < 0 ? 2 : 1, 200);
            if (n < 0 && errno != EINTR)
                break;
            if (f[0].revents & POLLIN) {
                while (read(tube_signal[0], &c, 1) == 1) {
                }
                if (t_sig < 0)
                    t_sig = maintenant_ms() - t0;
                // Comme ProcessMonitor : vider TOUS les fils termines.
                for (;;) {
                    int st = 0;
                    pid_t r = waitpid(-1, &st, WNOHANG);
                    if (r <= 0)
                        break;
                    if (r == p) {
                        rendu = r;
                        statut = st;
                    }
                }
            }
            if (t_eof < 0 && (f[1].revents & (POLLIN | POLLHUP))) {
                char b;
                if (read(prise[0], &b, 1) == 0)
                    t_eof = maintenant_ms() - t0;
            }
        }
        close(prise[0]);
        int ok = rendu == p && WIFSIGNALED(statut) && WTERMSIG(statut) == SIGSEGV && t_sig >= 0;
        printf("SIGCHLD_CAS cas=%s eof_ms=%ld sigchld_ms=%ld waitpid=%d statut=%s%d %s\n", noms[cas], t_eof, t_sig,
            (int)rendu, WIFSIGNALED(statut) ? "signal:" : "sortie:", WIFSIGNALED(statut) ? WTERMSIG(statut) : WEXITSTATUS(statut),
            ok ? "ok" : "ECHEC");
        if (!ok) {
            echecs++;
            // Ne pas laisser un fils a moitie mort fausser le cas suivant.
            kill(p, SIGKILL);
            waitpid(p, NULL, 0);
        }
    }
    if (echecs == 0)
        printf("SIGCHLD_MULTIFIL_OK\n");
    else
        printf("SIGCHLD_MULTIFIL_ECHEC n=%d\n", echecs);
    return echecs != 0;
}
