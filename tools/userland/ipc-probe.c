// Contre-pression : ce qui separe un canal d'un tampon qui grossit.
//
// Un tube et une paire de sockets sont bornes. Quand le tampon est plein,
// l'ecrivain doit l'apprendre — sinon il prend une avance illimitee sur son
// lecteur et fait grossir la memoire du noyau jusqu'a l'epuiser. C'est
// exactement la promesse de `poll(POLLOUT)` : « il reste de la place ».
//
// Ce noyau repondait « oui » a chaque fois, sans regarder. Un protocole IPC
// bati la-dessus n'aurait aucun moyen de ralentir : c'est la difference entre
// un canal et une fuite.
//
// La sonde etablit les quatre etats qui font un vrai canal :
//
//   1. canal vide       → POLLOUT, et l'ecriture passe ;
//   2. canal sature     → pas de POLLOUT, et l'ecriture non bloquante rend
//                         EAGAIN ;
//   3. le lecteur vide  → POLLOUT revient, l'ecriture repasse ;
//   4. le lecteur ferme → POLLERR/POLLHUP, et l'ecriture rend EPIPE.
//
// Le quatrieme cas est le plus important des quatre : sans lui, un producteur
// dont le consommateur est mort voit un canal qui se vide jamais et boucle pour
// toujours.
//
//   musl-gcc -O2 -static-pie ipc-probe.c -o ipc-probe
//   (ou via ./build.sh musl)

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static int echecs = 0;
static int reussites = 0;

static void verifie(const char *nom, int condition, const char *detail)
{
    if (condition) {
        reussites++;
    } else {
        echecs++;
        printf("  - %s%s%s\n", nom, detail ? " : " : "", detail ? detail : "");
    }
}

// Les evenements que `poll` rend pour ce descripteur, sans attendre.
static short interroge(int fd, short demandes)
{
    struct pollfd p = { .fd = fd, .events = demandes, .revents = 0 };
    if (poll(&p, 1, 0) < 0)
        return -1;
    return p.revents;
}

// Remplit le canal jusqu'a saturation. Rend le nombre d'octets ecrits, ou -1 si
// le canal accepte plus que de raison — ce qui voudrait dire qu'il n'est pas
// borne du tout.
static long sature(int fd)
{
    // Un mebioctet de marge au-dessus de la capacite attendue (64 KiB). Si le
    // canal avale tout cela sans broncher, il n'a pas de plafond.
    const long plafond = 1024L * 1024L;
    char bloc[4096];
    memset(bloc, 'x', sizeof bloc);
    long total = 0;
    while (total < plafond) {
        ssize_t n = write(fd, bloc, sizeof bloc);
        if (n < 0) {
            if (errno == EAGAIN)
                return total;
            return -2;
        }
        if (n == 0)
            return total;
        total += n;
    }
    return -1;
}

static void verifie_canal(const char *quoi, int lecture, int ecriture)
{
    char detail[160];

    // 1. Canal vide : de la place, et `poll` le dit.
    short r = interroge(ecriture, POLLOUT);
    snprintf(detail, sizeof detail, "%s: revents=0x%x", quoi, (unsigned)r);
    verifie("canal vide : POLLOUT", (r & POLLOUT) != 0, detail);

    // 2. Saturation. C'est le cœur du test : sans plafond, cette boucle
    //    n'aurait pas de fin utile.
    long ecrits = sature(ecriture);
    snprintf(detail, sizeof detail, "%s: %ld octets avales", quoi, ecrits);
    verifie("le canal a une capacite bornee", ecrits > 0, detail);
    if (ecrits <= 0)
        return;

    r = interroge(ecriture, POLLOUT);
    snprintf(detail, sizeof detail, "%s: revents=0x%x apres %ld octets",
             quoi, (unsigned)r, ecrits);
    verifie("canal sature : plus de POLLOUT", (r & POLLOUT) == 0, detail);

    // L'ecriture non bloquante doit refuser, pas mentir sur ce qu'elle a ecrit.
    char un = 'z';
    ssize_t refus = write(ecriture, &un, 1);
    snprintf(detail, sizeof detail, "%s: write=%zd errno=%d", quoi, refus, errno);
    verifie("canal sature : l'ecriture rend EAGAIN",
            refus < 0 && errno == EAGAIN, detail);

    // 3. Le lecteur consomme.
    //
    // On vide **entierement**, et non « quelques octets ». Un noyau a le droit
    // de n'annoncer `POLLOUT` qu'au-dela d'un seuil de place libre — c'est le
    // `SO_SNDLOWAT` de Linux, qui evite de reveiller un producteur pour lui
    // offrir trois octets. Exiger le retour de `POLLOUT` apres une lecture de
    // 8 KiB dans un tampon de 200 KiB testerait donc une regle que le noyau de
    // reference ne suit pas. Ce qui doit etre vrai partout, et qu'on verifie
    // ici, c'est qu'un canal vide est ecrivable.
    char vidage[8192];
    long lus = 0;
    for (;;) {
        ssize_t n = read(lecture, vidage, sizeof vidage);
        if (n <= 0)
            break;
        lus += n;
    }
    snprintf(detail, sizeof detail, "%s: %ld octets relus", quoi, lus);
    verifie("le lecteur vide le canal", lus > 0, detail);

    // 4. Aucun octet perdu : la contre-pression doit ralentir l'ecrivain, pas
    //    jeter ce qu'il a ecrit.
    snprintf(detail, sizeof detail, "%s: %ld ecrits, %ld relus", quoi, ecrits, lus);
    verifie("aucun octet n'est perdu par la contre-pression", lus == ecrits, detail);

    r = interroge(ecriture, POLLOUT);
    snprintf(detail, sizeof detail, "%s: revents=0x%x", quoi, (unsigned)r);
    verifie("canal vide de nouveau : POLLOUT revient", (r & POLLOUT) != 0, detail);

    ssize_t reprise = write(ecriture, &un, 1);
    snprintf(detail, sizeof detail, "%s: write=%zd errno=%d", quoi, reprise, errno);
    verifie("canal vide de nouveau : l'ecriture repasse", reprise == 1, detail);
    if (reprise == 1)
        (void)!read(lecture, vidage, sizeof vidage);
}

// Le lecteur ferme : l'ecrivain doit l'apprendre, et vite.
static void verifie_lecteur_ferme(const char *quoi, int lecture, int ecriture)
{
    char detail[160];
    close(lecture);

    short r = interroge(ecriture, POLLOUT);
    snprintf(detail, sizeof detail, "%s: revents=0x%x", quoi, (unsigned)r);
    // `POLLHUP` suffit a signaler la rupture ; `POLLERR` la qualifie en erreur
    // du cote ecriture. On exige au moins l'un des deux.
    verifie("lecteur ferme : poll signale la rupture",
            (r & (POLLHUP | POLLERR)) != 0, detail);

    char un = 'z';
    ssize_t refus = write(ecriture, &un, 1);
    snprintf(detail, sizeof detail, "%s: write=%zd errno=%d", quoi, refus, errno);
    verifie("lecteur ferme : l'ecriture rend EPIPE",
            refus < 0 && errno == EPIPE, detail);

    close(ecriture);
}

int main(void)
{
    printf("ipc-probe: contre-pression des canaux locaux\n");

    // Ecrire dans un canal sans lecteur leve `SIGPIPE`, dont l'action par
    // defaut est de tuer le processus. On veut mesurer l'`EPIPE` que rend
    // l'appel, pas mourir avant de l'avoir lu — c'est exactement ce que font
    // les vrais programmes qui parlent sur des sockets.
    signal(SIGPIPE, SIG_IGN);

    // --- Tube ---------------------------------------------------------------
    int tube[2];
    if (pipe(tube) != 0) {
        printf("RESULTAT : pipe() a echoue, rien a mesurer\n");
        return 1;
    }
    // Non bloquant des deux cotes : un test qui bloque n'est pas un test, c'est
    // une machine figee.
    fcntl(tube[0], F_SETFL, O_NONBLOCK);
    fcntl(tube[1], F_SETFL, O_NONBLOCK);
    verifie_canal("tube", tube[0], tube[1]);
    verifie_lecteur_ferme("tube", tube[0], tube[1]);

    // --- Paire de sockets ---------------------------------------------------
    int paire[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, paire) != 0) {
        printf("  (socketpair indisponible, moitie du test sautee)\n");
        echecs++;
    } else {
        fcntl(paire[0], F_SETFL, O_NONBLOCK);
        fcntl(paire[1], F_SETFL, O_NONBLOCK);
        // L'extremite 1 ecrit, l'extremite 0 lit.
        verifie_canal("socketpair", paire[0], paire[1]);
        verifie_lecteur_ferme("socketpair", paire[0], paire[1]);
    }

    printf("RESULTAT : %d verification(s) en echec (%d passees)\n",
           echecs, reussites);
    return echecs == 0 ? 0 : 1;
}
