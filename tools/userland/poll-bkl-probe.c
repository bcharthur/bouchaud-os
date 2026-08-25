// Sonde de vivacite poll/BKL.
//
// LE DEFAUT QU'ELLE REPRODUIT
// ---------------------------
// Sur CPU4, avec le vrai Ladybird, le noyau gelait : un CPU gardait le gros
// verrou pendant plus de quatre-vingts secondes, dans l'appel systeme 7
// (`poll`), et plus aucune acquisition n'avait lieu nulle part.
//
//     owner=1  cpu=0  task=17  syscall=7:2  held=69423ms
//     gen=952929  acq=952929  rel=952928     <- figes
//     switches=139238                        <- fige
//
// Le chemin : `poll` -> `readable(fd)` -> `socket_readable` -> `TcpConn::pump`
// -> `net::send_ip` -> `hop_mac` -> `arp_resolve`. Et `arp_resolve` est une
// attente serree bornee par l'HORLOGE, pas par un evenement : quatre tentatives
// de 500 ms, sans jamais ceder le processeur ni le verrou. Deux secondes de
// noyau gele par paquet a emettre vers un voisin qui ne repond pas -- et
// `pump` emet un accuse par segment recu.
//
// CE QU'ELLE MESURE
// -----------------
// Un fil declenche le piege : il ecrit vers une adresse du meme sous-reseau
// que personne n'occupe, donc dont l'ARP ne se resout jamais. Trois autres
// fils appellent en boucle un appel systeme resté SOUS le gros verrou et
// chronometrent chaque appel. Si un CPU gele le verrou, leur appel le plus
// long le montre directement, en microsecondes.
//
// Le verdict cote noyau est dans `[BKL-STATS] max_hold_ns=` : c'est la plus
// longue tenue CONTINUE du verrou. Un cumul ne dirait rien -- mille tenues
// d'une microseconde et une tenue de deux secondes donnent la meme somme.
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <pthread.h>
#include <unistd.h>
#include <time.h>
#include <fcntl.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <poll.h>

// 10.0.2.99 : dans le sous-reseau de QEMU en mode utilisateur, et personne ne
// l'occupe. Une requete ARP pour cette adresse reste donc sans reponse.
#define VOISIN_MUET "10.0.2.99"

static volatile int fini = 0;
static long long pire_us = 0;
static long long appels = 0;

static long long maintenant_us(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000000 + ts.tv_nsec / 1000;
}

// Le piege : un socket UDP vers un voisin muet, interroge par `poll` puis
// pousse par `sendto`. Les deux passent par la resolution ARP.
static void *fil_piege(void *arg)
{
    (void)arg;
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (fd < 0) { printf("POLL_BKL_ECHEC socket\n"); fini = 1; return NULL; }
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(9999);
    a.sin_addr.s_addr = inet_addr(VOISIN_MUET);

    for (int i = 0; i < 6; ++i) {
        sendto(fd, "x", 1, 0, (struct sockaddr *)&a, sizeof a);
        struct pollfd p = { .fd = fd, .events = POLLIN };
        poll(&p, 1, 50);
    }
    close(fd);
    fini = 1;
    return NULL;
}

// Les temoins : un appel systeme resté sous le gros verrou, chronometre.
// `fcntl(F_GETFD)` sur un descripteur ferme est bon marche, echoue proprement,
// et n'est PAS dans la table SANS_BKL.
static void *fil_temoin(void *arg)
{
    (void)arg;
    while (!fini) {
        long long t0 = maintenant_us();
        fcntl(-1, F_GETFD);
        long long d = maintenant_us() - t0;
        __sync_fetch_and_add(&appels, 1);
        if (d > pire_us) pire_us = d;
    }
    return NULL;
}

int main(void)
{
    printf("poll-bkl-probe: piege ARP vers %s, 3 temoins sous BKL\n", VOISIN_MUET);
    pthread_t piege, temoins[3];
    if (pthread_create(&piege, NULL, fil_piege, NULL) != 0) {
        printf("POLL_BKL_ECHEC pthread piege\n");
        return 1;
    }
    int n = 0;
    for (int i = 0; i < 3; ++i)
        if (pthread_create(&temoins[i], NULL, fil_temoin, NULL) == 0) ++n;

    pthread_join(piege, NULL);
    for (int i = 0; i < n; ++i) pthread_join(temoins[i], NULL);

    printf("POLL_BKL_PIRE_US %lld appels=%lld temoins=%d\n", pire_us, appels, n);
    if (n != 3) { printf("POLL_BKL_ECHEC temoins=%d\n", n); return 1; }
    if (appels == 0) { printf("POLL_BKL_ECHEC aucun appel temoin\n"); return 1; }
    // Un appel systeme trivial qui met plus d'un quart de seconde veut dire que
    // le verrou global a ete tenu tout ce temps par quelqu'un d'autre.
    if (pire_us > 250000) {
        printf("POLL_BKL_ECHEC verrou global tenu %lld us\n", pire_us);
        return 1;
    }
    printf("POLL_BKL_OK\n");
    return 0;
}
