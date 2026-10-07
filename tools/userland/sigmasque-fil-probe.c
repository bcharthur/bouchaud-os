/*
 * Sonde : le masque des signaux appartient au FIL, pas au processus.
 *
 * BOUCHAUD_SIGMASQUE_PAR_FIL_V1
 *
 * POSIX (et Linux) : `pthread_sigmask`/`rt_sigprocmask` change le masque du
 * fil appelant ; un nouveau fil herite du masque de son createur ; un signal
 * de processus va a un fil qui ne le bloque pas. Bouchaud gardait UN masque
 * par processus. La glibc bloque tout autour de `pthread_create` et de
 * `posix_spawn`, puis restaure le masque qu'elle a sauve : deux fils qui le
 * font en meme temps -- ou un fil qui le fait pendant qu'un autre execute un
 * gestionnaire (signal masque le temps du gestionnaire) -- et le masque
 * restaure est celui, transitoire, de l'autre. SIGCHLD reste bloque pour
 * toujours. Run 37627107473 : le navigateur recolte la PREMIERE mort d'un
 * fils (WebWorker tue), plus aucune ensuite (endurance : WebContent 11 crees,
 * 0 recoltes ; WebWorker 29 / 1) alors que le noyau les voit sortir.
 *
 * Cas :
 *   isolement  un fil secondaire bloque SIGUSR1 ; le masque du fil principal
 *              ne change pas, et kill(getpid(), SIGUSR1) est livre (au fil
 *              principal) en moins de 3 s ;
 *   heritage   le fil principal bloque SIGUSR2 puis cree un fil : le fil cree
 *              le trouve bloque ; le fil principal le debloque, le fil cree
 *              le garde ;
 *   course     un fil secondaire bloque tout / restaure en boucle (comme la
 *              glibc autour de pthread_create) pendant que le fil principal
 *              cree 20 fils qui sortent par exit(0) : chacun doit etre
 *              recolte sur SIGCHLD (gestionnaire -> tube -> waitpid) en
 *              moins de 3 s.
 * Sortie : une ligne par cas, puis SIGMASQUE_FIL_OK ou SIGMASQUE_FIL_ECHEC n=...
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int tube[2];
static volatile int continuer = 1;
static volatile pid_t fil_de_livraison;

static long maintenant_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

static void gestionnaire(int sig)
{
    int e = errno;
    char c = (char)sig;
    fil_de_livraison = gettid();
    (void)!write(tube[1], &c, 1);
    errno = e;
}

static int bloque(int sig)
{
    sigset_t cur;
    pthread_sigmask(SIG_BLOCK, NULL, &cur);
    return sigismember(&cur, sig);
}

/* Attend un octet du gestionnaire (au plus `ms`). */
static int attend_signal(int ms)
{
    long t0 = maintenant_ms();
    for (;;) {
        long reste = ms - (maintenant_ms() - t0);
        if (reste <= 0)
            return 0;
        struct pollfd p = { .fd = tube[0], .events = POLLIN };
        int n = poll(&p, 1, (int)reste);
        if (n > 0) {
            char c;
            while (read(tube[0], &c, 1) == 1) {
            }
            return 1;
        }
        if (n < 0 && errno != EINTR)
            return 0;
    }
}

static pthread_barrier_t barriere;

static void *bloque_usr1(void *arg)
{
    (void)arg;
    sigset_t s;
    sigemptyset(&s);
    sigaddset(&s, SIGUSR1);
    pthread_sigmask(SIG_BLOCK, &s, NULL);
    pthread_barrier_wait(&barriere); /* masque pose */
    pthread_barrier_wait(&barriere); /* le principal a fini */
    return NULL;
}

static void *rapporte_usr2(void *arg)
{
    int *vu = arg;
    vu[0] = bloque(SIGUSR2);
    pthread_barrier_wait(&barriere); /* lu a la naissance */
    pthread_barrier_wait(&barriere); /* le principal a debloque */
    vu[1] = bloque(SIGUSR2);
    return NULL;
}

static void *bascule_tout(void *arg)
{
    (void)arg;
    sigset_t tout, ancien;
    sigfillset(&tout);
    while (continuer) {
        pthread_sigmask(SIG_SETMASK, &tout, &ancien);
        usleep(300);
        pthread_sigmask(SIG_SETMASK, &ancien, NULL);
        usleep(100);
    }
    return NULL;
}

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    if (pipe2(tube, O_CLOEXEC | O_NONBLOCK) != 0) {
        printf("SIGMASQUE_FIL_ECHEC n=1 raison=pipe2\n");
        return 1;
    }
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = gestionnaire;
    sa.sa_flags = SA_RESTART;
    sigaction(SIGUSR1, &sa, NULL);
    sigaction(SIGCHLD, &sa, NULL);
    int echecs = 0;

    /* isolement */
    {
        pthread_barrier_init(&barriere, NULL, 2);
        pthread_t t;
        pthread_create(&t, NULL, bloque_usr1, NULL);
        pthread_barrier_wait(&barriere);
        int principal_bloque = bloque(SIGUSR1);
        fil_de_livraison = 0;
        kill(getpid(), SIGUSR1);
        int livre = attend_signal(3000);
        int ok = !principal_bloque && livre && fil_de_livraison == gettid();
        printf("SIGMASQUE_CAS cas=isolement principal_bloque=%d livre=%d fil_principal=%d %s\n", principal_bloque, livre,
            fil_de_livraison == gettid(), ok ? "ok" : "ECHEC");
        echecs += !ok;
        pthread_barrier_wait(&barriere);
        pthread_join(t, NULL);
        pthread_barrier_destroy(&barriere);
    }

    /* heritage */
    {
        pthread_barrier_init(&barriere, NULL, 2);
        sigset_t s;
        sigemptyset(&s);
        sigaddset(&s, SIGUSR2);
        pthread_sigmask(SIG_BLOCK, &s, NULL);
        int vu[2] = { -1, -1 };
        pthread_t t;
        pthread_create(&t, NULL, rapporte_usr2, vu);
        pthread_barrier_wait(&barriere);
        pthread_sigmask(SIG_UNBLOCK, &s, NULL);
        pthread_barrier_wait(&barriere);
        pthread_join(t, NULL);
        int principal = bloque(SIGUSR2);
        int ok = vu[0] == 1 && vu[1] == 1 && principal == 0;
        printf("SIGMASQUE_CAS cas=heritage fil_a_la_naissance=%d fil_apres=%d principal_apres=%d %s\n", vu[0], vu[1],
            principal, ok ? "ok" : "ECHEC");
        echecs += !ok;
        pthread_barrier_destroy(&barriere);
    }

    /* course */
    {
        continuer = 1;
        pthread_t t;
        pthread_create(&t, NULL, bascule_tout, NULL);
        int recoltes = 0, pire = 0;
        for (int i = 0; i < 20; i++) {
            long t0 = maintenant_ms();
            pid_t p = fork();
            if (p == 0) {
                usleep(20 * 1000);
                exit(0);
            }
            pid_t rendu = 0;
            while (rendu != p && maintenant_ms() - t0 < 3000) {
                if (!attend_signal((int)(3000 - (maintenant_ms() - t0))))
                    break;
                int st;
                pid_t r;
                while ((r = waitpid(-1, &st, WNOHANG)) > 0) {
                    if (r == p)
                        rendu = r;
                }
            }
            long d = maintenant_ms() - t0;
            if (rendu == p) {
                recoltes++;
                if (d > pire)
                    pire = (int)d;
            } else {
                waitpid(p, NULL, 0);
            }
        }
        continuer = 0;
        pthread_join(t, NULL);
        int ok = recoltes == 20;
        printf("SIGMASQUE_CAS cas=course recoltes_sur_sigchld=%d/20 pire_ms=%d principal_bloque_sigchld=%d %s\n", recoltes,
            pire, bloque(SIGCHLD), ok ? "ok" : "ECHEC");
        echecs += !ok;
    }

    if (echecs == 0)
        printf("SIGMASQUE_FIL_OK\n");
    else
        printf("SIGMASQUE_FIL_ECHEC n=%d\n", echecs);
    return echecs != 0;
}
