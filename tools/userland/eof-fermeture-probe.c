/*
 * eof-fermeture-probe : la fin de fichier (et EPIPE) doit arriver des que la
 * DERNIERE extremite d'un tube ou d'une paire de sockets disparait -- quel que
 * soit le chemin qui la fait disparaitre.
 *
 * POURQUOI CETTE SONDE
 *
 * Un lecteur bloque dans `read` sur un tube vide dort dans la file de
 * disponibilite (`READINESS`). Il n'en sort que si quelqu'un appelle
 * `notify_readiness`. `close()` le fait ; la mort d'un processus aussi, mais
 * seulement au travers du SIGCHLD de son pere. Trois autres chemins ferment
 * une extremite sans rien reveiller :
 *
 *   - `execve` ferme les descripteurs FD_CLOEXEC. C'est exactement le motif
 *     de `posix_spawn` : le pere lit un tube CLOEXEC, la fin de fichier dit
 *     « l'exec a reussi » ;
 *   - `dup2` / `dup3` vers un numero deja ouvert ferment l'ancien
 *     descripteur ;
 *   - la mort d'un orphelin : son pere n'existe plus, aucun SIGCHLD n'est
 *     emis, rien ne reveille.
 *
 * Le lecteur ne voit alors la fin de fichier qu'au prochain evenement sans
 * rapport -- ou jamais, sur un systeme calme.
 *
 * LA MESURE
 *
 * Le fils ferme sa derniere extremite PUIS dort trois secondes sans aucune
 * entree/sortie. La fin de fichier (ou EPIPE) doit arriver pendant ce sommeil :
 * si le fils est deja mort quand le pere la recoit, elle n'est venue qu'avec
 * sa mort -- le reveil de la fermeture a ete perdu. Et elle doit arriver vite
 * (BORNE_FERMETURE_MS) : un reveil venu d'ailleurs n'est pas le sien. Pour
 * l'orphelin, dont la fermeture EST la mort, la borne est un delai : une
 * alarme a 5 s fait office de reveil tardif et la sonde mesure qui est arrive
 * le premier.
 *
 *     eof-fermeture-probe [TOURS]
 *
 * Verdict : `EOF_FERMETURE_OK cas=N tours=T` ; sinon `EOF_FERMETURE_FAIL` et
 * code 1. Tourne aussi sur Linux (reference).
 *
 *   musl-gcc -O2 -static-pie eof-fermeture-probe.c -o eof-fermeture-probe
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define SOMMEIL_FILS_MS 3000
#define DELAI_AVANT_FERMETURE_MS 150
/* Fermeture + exec : 0,15 s sur Linux. Au-dela, un autre evenement a fait le
 * reveil (la lecture bloquante d'un socketpair a un filet de 2 s). */
#define BORNE_FERMETURE_MS 1500
#define ORPHELIN_MEURT_MS 300
#define ORPHELIN_BORNE_MS 2500
#define ALARME_S 5

static int echecs = 0;
static const char *moi = "/bin/eof-fermeture-probe";

static long long maintenant_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

static void dort_ms(long ms)
{
    struct timespec t = { ms / 1000, (ms % 1000) * 1000000L };
    while (nanosleep(&t, &t) != 0 && errno == EINTR)
        ;
}

static void sur_alarme(int s) { (void)s; }

/* Le fils dormeur, apres exec : aucune entree/sortie, puis la sortie. */
static int dormeur(void)
{
    dort_ms(SOMMEIL_FILS_MS);
    return 0;
}

/* Le fils, juste avant de fermer : laisser au pere le temps de se bloquer. */
static void fils_puis_exec(void)
{
    dort_ms(DELAI_AVANT_FERMETURE_MS);
    char *argv[] = { (char *)moi, NULL };
    char *envp[] = { "EOF_FERMETURE_DORMEUR=1", NULL };
    execve(moi, argv, envp);
    _exit(127);
}

/*
 * Rend 1 si le fils `pid` vit encore (le pere a recu sa reponse PENDANT le
 * sommeil du fils), 0 s'il est deja mort. Recolte toujours le fils.
 */
static int vivant_puis_recolte(pid_t pid, int *statut)
{
    int vivant = waitpid(pid, statut, WNOHANG) == 0;
    if (vivant)
        waitpid(pid, statut, 0);
    return vivant;
}

static void juge(const char *cas, int tour, long long rendu, int erreur, long long ms, int vivant,
                 int attendu_ok)
{
    printf("EOF_FERMETURE cas=%s tour=%d rendu=%lld errno=%d ms=%lld fils_vivant=%d %s\n", cas, tour,
           rendu, erreur, ms, vivant, attendu_ok ? "ok" : "ECHEC");
    if (!attendu_ok)
        echecs++;
}

/* Lecture bloquante ; le fils ferme la derniere extremite d'ecriture. */
static void cas_lecture(const char *cas, int tour, int socket, int mode)
{
    int c[2];
    int r = socket ? socketpair(AF_UNIX, SOCK_STREAM | (mode == 0 ? SOCK_CLOEXEC : 0), 0, c)
                   : pipe2(c, mode == 0 ? O_CLOEXEC : 0);
    if (r != 0) {
        printf("EOF_FERMETURE cas=%s creation errno=%d ECHEC\n", cas, errno);
        echecs++;
        return;
    }
    /* c[0] : le pere lit ; c[1] : la derniere extremite d'ecriture, au fils. */
    pid_t pid = fork();
    if (pid == 0) {
        close(c[0]);
        if (mode == 0)
            fils_puis_exec();
        dort_ms(DELAI_AVANT_FERMETURE_MS);
        if (mode == 1)
            dup2(2, c[1]);
        else
            dup3(2, c[1], 0);
        dort_ms(SOMMEIL_FILS_MS);
        _exit(0);
    }
    close(c[1]);
    long long t0 = maintenant_ms();
    char octet;
    long long n = read(c[0], &octet, 1);
    int erreur = n < 0 ? errno : 0;
    long long ms = maintenant_ms() - t0;
    int statut = 0;
    int vivant = vivant_puis_recolte(pid, &statut);
    close(c[0]);
    juge(cas, tour, n, erreur, ms, vivant, n == 0 && vivant && ms < BORNE_FERMETURE_MS);
}

/* Ecriture bloquante sur un tube plein ; le fils ferme le dernier lecteur. */
static void cas_ecriture(const char *cas, int tour)
{
    int c[2];
    if (pipe2(c, O_CLOEXEC) != 0) {
        printf("EOF_FERMETURE cas=%s creation errno=%d ECHEC\n", cas, errno);
        echecs++;
        return;
    }
    pid_t pid = fork();
    if (pid == 0) {
        close(c[1]);
        fils_puis_exec();
    }
    close(c[0]);
    /* Remplir sans bloquer, puis une ecriture bloquante qui attend de la place. */
    int drapeaux = fcntl(c[1], F_GETFL);
    fcntl(c[1], F_SETFL, drapeaux | O_NONBLOCK);
    char bloc[4096];
    memset(bloc, 'x', sizeof bloc);
    while (write(c[1], bloc, sizeof bloc) > 0)
        ;
    fcntl(c[1], F_SETFL, drapeaux);
    long long t0 = maintenant_ms();
    long long n = write(c[1], bloc, sizeof bloc);
    int erreur = n < 0 ? errno : 0;
    long long ms = maintenant_ms() - t0;
    int statut = 0;
    int vivant = vivant_puis_recolte(pid, &statut);
    close(c[1]);
    juge(cas, tour, n, erreur, ms, vivant,
         n < 0 && erreur == EPIPE && vivant && ms < BORNE_FERMETURE_MS);
}

/*
 * L'orphelin : le petit-fils tient l'extremite d'ecriture et meurt APRES son
 * pere. Personne ne recoit son SIGCHLD.
 */
static void cas_orphelin(const char *cas, int tour)
{
    int c[2];
    if (pipe(c) != 0) {
        printf("EOF_FERMETURE cas=%s creation errno=%d ECHEC\n", cas, errno);
        echecs++;
        return;
    }
    long long t0 = maintenant_ms();
    pid_t pid = fork();
    if (pid == 0) {
        if (fork() == 0) {
            close(c[0]);
            dort_ms(ORPHELIN_MEURT_MS);
            _exit(0);
        }
        _exit(0);
    }
    close(c[1]);
    int statut = 0;
    waitpid(pid, &statut, 0);
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = sur_alarme;
    sigaction(SIGALRM, &sa, NULL);
    alarm(ALARME_S);
    char octet;
    long long n = read(c[0], &octet, 1);
    int erreur = n < 0 ? errno : 0;
    alarm(0);
    long long ms = maintenant_ms() - t0;
    close(c[0]);
    juge(cas, tour, n, erreur, ms, -1, n == 0 && ms < ORPHELIN_BORNE_MS);
}

int main(int argc, char **argv)
{
    if (getenv("EOF_FERMETURE_DORMEUR"))
        return dormeur();
    if (argc > 0 && argv[0][0] == '/')
        moi = argv[0];
    else if (argc > 0 && strchr(argv[0], '/'))
        moi = realpath(argv[0], NULL);
    int tours = argc > 1 ? atoi(argv[1]) : 3;
    if (tours < 1 || !moi) {
        printf("EOF_FERMETURE_FAIL parametres\n");
        return 1;
    }
    signal(SIGPIPE, SIG_IGN);
    for (int t = 1; t <= tours; t++) {
        cas_lecture("exec-cloexec-tube", t, 0, 0);
        cas_lecture("exec-cloexec-socket", t, 1, 0);
        cas_lecture("dup2-tube", t, 0, 1);
        cas_lecture("dup3-socket", t, 1, 2);
        cas_ecriture("exec-cloexec-epipe", t);
        cas_orphelin("orphelin-tube", t);
    }
    if (echecs) {
        printf("EOF_FERMETURE_FAIL echecs=%d\n", echecs);
        return 1;
    }
    printf("EOF_FERMETURE_OK cas=6 tours=%d\n", tours);
    return 0;
}
