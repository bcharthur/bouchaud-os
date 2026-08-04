/*
 * Sonde POSIX : verifie les fonctionnalites systeme au-dela du simple ring 3 —
 * creation de processus, signaux, memoire partagee, reseau.
 *
 * Chaque bloc correspond a une chose qu'un logiciel reel fait sans y penser et
 * qui, si elle manque, se manifeste par un plantage difficile a relier a sa
 * cause.
 *
 *   musl-gcc -O2 -static-pie -pthread posix-probe.c -o posix-probe
 *   (ou via ./build.sh musl)
 */

#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-44s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

/* ------------------------------------------------------------ processus */

static void teste_fork_wait(void)
{
    pid_t moi = getpid();
    pid_t enfant = fork();
    verifie("fork() reussit", enfant >= 0);
    if (enfant < 0)
        return;

    if (enfant == 0) {
        /* Cote enfant : espace d'adressage distinct du parent. */
        _exit(42);
    }

    verifie("fork() renvoie un pid different", enfant != moi && enfant > 0);

    int statut = 0;
    pid_t recolte = waitpid(enfant, &statut, 0);
    verifie("waitpid() recolte l'enfant", recolte == enfant);
    verifie("code de sortie transmis (42)", WIFEXITED(statut) && WEXITSTATUS(statut) == 42);

    /* Un second wait doit dire « plus d'enfants », pas rendre le meme. */
    verifie("waitpid() sans enfant -> ECHILD",
            waitpid(-1, &statut, 0) < 0 && errno == ECHILD);
}

static void teste_isolation_memoire(void)
{
    /* Le parent modifie une variable apres le fork : l'enfant ne doit rien en
     * voir, c'est toute la difference entre un processus et un thread. */
    static volatile int partagee = 1;
    int tube[2];
    if (pipe(tube) != 0) {
        verifie("pipe() pour le test d'isolation", 0);
        return;
    }

    pid_t enfant = fork();
    if (enfant == 0) {
        close(tube[0]);
        struct timespec pause = {0, 200 * 1000 * 1000};
        nanosleep(&pause, NULL);
        int vue = partagee;
        ssize_t ecrit = write(tube[1], &vue, sizeof vue);
        (void)ecrit;
        _exit(0);
    }
    close(tube[1]);
    partagee = 99; /* apres le fork : invisible pour l'enfant */

    int vue = -1;
    /* Le tube est partage par le fork : c'est ce qui permet de recuperer la
     * valeur observee par l'enfant. */
    for (int essai = 0; essai < 200 && vue == -1; essai++) {
        ssize_t lu = read(tube[0], &vue, sizeof vue);
        if (lu == sizeof vue)
            break;
        vue = -1;
        struct timespec pause = {0, 10 * 1000 * 1000};
        nanosleep(&pause, NULL);
    }
    close(tube[0]);
    waitpid(enfant, NULL, 0);

    printf("    l'enfant a vu %d, le parent a ecrit 99\n", vue);
    verifie("espaces d'adressage separes apres fork", vue == 1);
    verifie("tube herite par l'enfant", vue != -1);
}

static void teste_exec(void)
{
    pid_t enfant = fork();
    if (enfant < 0) {
        verifie("fork() avant execve", 0);
        return;
    }
    if (enfant == 0) {
        char *argv[] = {"/exec-cible", "bonjour", NULL};
        char *envp[] = {"MARQUEUR=transmis", NULL};
        execve("/exec-cible", argv, envp);
        /* On n'arrive ici que si execve a echoue. */
        _exit(127);
    }
    int statut = 0;
    waitpid(enfant, &statut, 0);
    printf("    /exec-cible a rendu %d\n", WEXITSTATUS(statut));
    verifie("execve() remplace l'image du processus",
            WIFEXITED(statut) && WEXITSTATUS(statut) == 7);
}

/* -------------------------------------------------------------- signaux */

static volatile sig_atomic_t recus = 0;
static volatile sig_atomic_t dernier = 0;

static void gestionnaire(int signal)
{
    recus++;
    dernier = signal;
}

static volatile sig_atomic_t alarme = 0;
static void sur_alarme(int signal)
{
    (void)signal;
    alarme = 1;
}

static void teste_signaux(void)
{
    struct sigaction action;
    memset(&action, 0, sizeof action);
    action.sa_handler = gestionnaire;
    sigemptyset(&action.sa_mask);
    verifie("sigaction(SIGUSR1)", sigaction(SIGUSR1, &action, NULL) == 0);

    /* Un signal qu'on s'envoie a soi-meme doit detourner l'execution vers le
     * gestionnaire, puis revenir exactement ici. */
    int temoin = 0x5A5A;
    verifie("raise(SIGUSR1)", raise(SIGUSR1) == 0);
    verifie("gestionnaire appele", recus == 1 && dernier == SIGUSR1);
    verifie("execution reprise apres le gestionnaire", temoin == 0x5A5A);

    /* Blocage : le signal doit rester en attente, puis etre livre au
     * deblocage — c'est ce que fait toute section critique. */
    sigset_t masque, ancien;
    sigemptyset(&masque);
    sigaddset(&masque, SIGUSR1);
    verifie("sigprocmask(SIG_BLOCK)", sigprocmask(SIG_BLOCK, &masque, &ancien) == 0);
    raise(SIGUSR1);
    verifie("signal bloque non livre", recus == 1);
    verifie("sigprocmask(SIG_UNBLOCK)", sigprocmask(SIG_UNBLOCK, &masque, NULL) == 0);
    verifie("signal livre au deblocage", recus == 2);

    /* SIG_IGN : le signal disparait sans rien declencher. */
    memset(&action, 0, sizeof action);
    action.sa_handler = SIG_IGN;
    sigaction(SIGUSR2, &action, NULL);
    raise(SIGUSR2);
    verifie("SIG_IGN : signal ignore", recus == 2);

    /* Un signal envoye a un autre processus le termine. */
    pid_t enfant = fork();
    if (enfant == 0) {
        for (;;)
            pause();
    }
    struct timespec pause_courte = {0, 100 * 1000 * 1000};
    nanosleep(&pause_courte, NULL);
    verifie("kill(enfant, SIGTERM)", kill(enfant, SIGTERM) == 0);
    int statut = 0;
    waitpid(enfant, &statut, 0);
    verifie("l'enfant est mort du signal", WIFSIGNALED(statut) || WEXITSTATUS(statut) != 0);

    /* alarm() : SIGALRM differe par le noyau. */
    memset(&action, 0, sizeof action);
    action.sa_handler = sur_alarme;
    sigaction(SIGALRM, &action, NULL);
    alarm(1);
    for (int i = 0; i < 300 && !alarme; i++) {
        struct timespec pause = {0, 10 * 1000 * 1000};
        nanosleep(&pause, NULL);
    }
    verifie("alarm(1) leve SIGALRM", alarme == 1);
}

/* ------------------------------------------------------- memoire partagee */

static void teste_mmap_partage(void)
{
    const char *chemin = "/partage.bin";
    int fd = open(chemin, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        verifie("open() du fichier de partage", 0);
        return;
    }
    char initial[4096];
    memset(initial, '.', sizeof initial);
    ssize_t ecrit = write(fd, initial, sizeof initial);
    (void)ecrit;

    char *carte = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    verifie("mmap(MAP_SHARED) sur fichier", carte != MAP_FAILED);
    if (carte == MAP_FAILED) {
        close(fd);
        return;
    }
    verifie("contenu du fichier visible dans la carte", carte[0] == '.');

    pid_t enfant = fork();
    if (enfant == 0) {
        /* L'enfant ecrit dans SA carte : le partage veut dire que le parent
         * le voit, alors qu'une variable ordinaire resterait invisible. */
        memcpy(carte, "ECRIT-PAR-L-ENFANT", 18);
        msync(carte, 4096, MS_SYNC);
        _exit(0);
    }
    waitpid(enfant, NULL, 0);

    verifie("ecriture de l'enfant visible par le parent",
            memcmp(carte, "ECRIT-PAR-L-ENFANT", 18) == 0);

    /* Et le fichier lui-meme doit avoir change. */
    msync(carte, 4096, MS_SYNC);
    char relu[32] = {0};
    lseek(fd, 0, SEEK_SET);
    ssize_t lu = read(fd, relu, 18);
    printf("    relecture du fichier : %.18s\n", relu);
    verifie("ecriture repercutee dans le fichier",
            lu == 18 && memcmp(relu, "ECRIT-PAR-L-ENFANT", 18) == 0);

    munmap(carte, 4096);
    close(fd);
}

/* --------------------------------------------------------------- reseau */

static void teste_reseau(void)
{
    /* Resolution de nom : la libc parle elle-meme au serveur DNS, en UDP. */
    struct addrinfo indices, *resultat = NULL;
    memset(&indices, 0, sizeof indices);
    indices.ai_family = AF_INET;
    indices.ai_socktype = SOCK_STREAM;

    int erreur = getaddrinfo("example.com", "80", &indices, &resultat);
    verifie("getaddrinfo(example.com)", erreur == 0 && resultat != NULL);
    if (erreur != 0 || resultat == NULL) {
        printf("    getaddrinfo a rendu %d (%s)\n", erreur, gai_strerror(erreur));
        return;
    }

    struct sockaddr_in *adresse = (struct sockaddr_in *)resultat->ai_addr;
    char texte[INET_ADDRSTRLEN] = {0};
    inet_ntop(AF_INET, &adresse->sin_addr, texte, sizeof texte);
    printf("    example.com -> %s\n", texte);

    int sock = socket(AF_INET, SOCK_STREAM, 0);
    verifie("socket(AF_INET, SOCK_STREAM)", sock >= 0);
    if (sock < 0) {
        freeaddrinfo(resultat);
        return;
    }

    verifie("connect() vers le port 80",
            connect(sock, resultat->ai_addr, resultat->ai_addrlen) == 0);

    const char *requete =
        "GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n";
    ssize_t envoye = send(sock, requete, strlen(requete), 0);
    verifie("send() de la requete HTTP", envoye == (ssize_t)strlen(requete));

    char reponse[512] = {0};
    ssize_t recu = recv(sock, reponse, sizeof reponse - 1, 0);
    printf("    %zd octets recus, debut : %.15s\n", recu, reponse);
    verifie("recv() rend une reponse HTTP",
            recu > 0 && memcmp(reponse, "HTTP/1.", 7) == 0);

    close(sock);
    freeaddrinfo(resultat);
}

static void teste_socketpair(void)
{
    int paire[2];
    verifie("socketpair(AF_UNIX)", socketpair(AF_UNIX, SOCK_STREAM, 0, paire) == 0);
    const char *message = "aller-retour";
    ssize_t ecrit = write(paire[0], message, strlen(message));
    char tampon[32] = {0};
    ssize_t lu = read(paire[1], tampon, sizeof tampon - 1);
    verifie("socketpair transmet dans un sens",
            ecrit > 0 && lu == ecrit && strcmp(tampon, message) == 0);

    /* Bidirectionnel : l'autre sens doit fonctionner aussi. */
    ecrit = write(paire[1], "retour", 6);
    memset(tampon, 0, sizeof tampon);
    lu = read(paire[0], tampon, sizeof tampon - 1);
    verifie("socketpair transmet dans l'autre sens",
            lu == 6 && memcmp(tampon, "retour", 6) == 0);
    close(paire[0]);
    close(paire[1]);
}

/* ----------------------------------------------------------------- main */

int main(void)
{
    printf("Sonde POSIX — processus, signaux, memoire partagee, reseau\n\n");

    printf("[processus]\n");
    teste_fork_wait();

    printf("\n[isolation memoire]\n");
    teste_isolation_memoire();

    printf("\n[execve]\n");
    teste_exec();

    printf("\n[signaux]\n");
    teste_signaux();

    printf("\n[mmap partage]\n");
    teste_mmap_partage();

    printf("\n[socketpair]\n");
    teste_socketpair();

    printf("\n[reseau]\n");
    teste_reseau();

    printf("\nRESULTAT : %d verification(s) en echec\n", echecs);
    return echecs == 0 ? 0 : 1;
}
