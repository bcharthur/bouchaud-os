/*
 * tube-atomicite-probe : plusieurs ecrivains sur un meme tube, et aucune
 * ecriture de PIPE_BUF octets ou moins n'est coupee ni entrelacee.
 *
 * POURQUOI CETTE SONDE
 *
 * POSIX : une ecriture de `n <= PIPE_BUF` octets dans un tube est ATOMIQUE --
 * ses octets ne sont jamais melanges a ceux d'un autre ecrivain. Bloquante,
 * elle attend qu'il y ait `n` octets de place ; non bloquante, elle rend
 * EAGAIN plutot qu'une ecriture partielle. Les journaux de plusieurs processus
 * dans un meme tube, le jeton d'un jobserver, tout protocole a messages courts
 * sur un tube partage reposent sur cette garantie.
 *
 * Le noyau rendait une ecriture partielle des qu'il restait UN octet de place,
 * et relisait la place hors du verrou du tube avant d'ecrire. Une ecriture
 * partielle se complete par une seconde, entre lesquelles un autre ecrivain
 * peut passer : l'enregistrement est coupe en deux.
 *
 * LA MESURE
 *
 * ECRIVAINS processus ecrivent chacun TOURS enregistrements de R octets
 * (octet d'identite repete, numero de sequence) ; le lecteur lit par tranches
 * irregulieres (pour que la place libre ne soit pas un multiple de R) et
 * decoupe le flux en enregistrements de R octets. Un enregistrement melange,
 * une sequence qui saute, un compte faux : ECHEC. R parcourt 100, 512 et
 * 4096 (= PIPE_BUF).
 *
 *     tube-atomicite-probe [TOURS]
 *
 * POLLOUT ET ECRITURE TOUT-OU-RIEN
 *
 * `poll` annonce POLLOUT quand une ecriture de PIPE_BUF octets passe
 * (semantique Linux). Annoncer POLLOUT des qu'il reste UN octet, face a une
 * ecriture tout-ou-rien qui rend EAGAIN, fait tourner une boucle d'evenements
 * non bloquante entre POLLOUT et EAGAIN -- ses messages s'accumulent en
 * memoire. Le tube est rempli par ecritures de 1000 octets jusqu'a EAGAIN,
 * puis vide par tranches de 700 : chaque fois que POLLOUT est annonce, une
 * ecriture de PIPE_BUF octets doit passer entiere ; tube vide, POLLOUT est du.
 *
 * Verdict : `TUBE_ATOMICITE_OK tailles=3 ecrivains=N tours=T` ; sinon
 * `TUBE_ATOMICITE_FAIL` et code 1. Tourne aussi sur Linux (reference).
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define ECRIVAINS 4
#define ENTETE 5 /* identite + sequence sur 4 octets */

static int echecs = 0;

static long long maintenant_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

/* Un ecrivain : `tours` enregistrements de `r` octets, chacun en UN write. */
static void ecrivain(int fd, int id, int tours, int r)
{
    unsigned char e[PIPE_BUF];
    int partielles = 0;
    for (int s = 0; s < tours; s++) {
        memset(e, 'A' + id, (size_t)r);
        e[1] = (unsigned char)(s & 0xff);
        e[2] = (unsigned char)((s >> 8) & 0xff);
        e[3] = (unsigned char)((s >> 16) & 0xff);
        e[4] = (unsigned char)((s >> 24) & 0xff);
        ssize_t n = write(fd, e, (size_t)r);
        if (n != r) {
            /* Une ecriture partielle d'au plus PIPE_BUF octets est deja la
             * faute ; la completer comme le ferait un programme ordinaire
             * laisse le lecteur voir l'entrelacement qu'elle permet. */
            if (n < 0)
                _exit(10);
            ssize_t fait = n;
            while (fait < r) {
                ssize_t m = write(fd, e + fait, (size_t)(r - fait));
                if (m < 0)
                    _exit(11);
                fait += m;
            }
            partielles++;
        }
    }
    /* Le nombre d'ecritures partielles, borne sous 128 : un statut de 128 a
     * 255 serait lu comme une mort par signal par un noyau qui confond les
     * deux (defaut a part). */
    _exit(partielles ? 100 + (partielles > 27 ? 27 : partielles) : 0);
}

static void une_taille(int r, int tours)
{
    int c[2];
    if (pipe(c) != 0) {
        printf("TUBE_ATOMICITE r=%d pipe errno=%d ECHEC\n", r, errno);
        echecs++;
        return;
    }
    pid_t fils[ECRIVAINS];
    for (int i = 0; i < ECRIVAINS; i++) {
        fils[i] = fork();
        if (fils[i] == 0) {
            close(c[0]);
            ecrivain(c[1], i, tours, r);
        }
    }
    close(c[1]);
    long long t0 = maintenant_ms();
    long attendus = (long)ECRIVAINS * tours * r;
    unsigned char *flux = malloc((size_t)attendus + PIPE_BUF);
    long lus = 0;
    /* Tranches irregulieres : jamais un multiple de r. */
    static const int tranches[] = { 1000, 333, 4097, 7, 2500 };
    for (int k = 0;; k++) {
        ssize_t n = read(c[0], flux + lus, (size_t)tranches[k % 5]);
        if (n <= 0)
            break;
        lus += n;
        if (lus > attendus)
            break;
    }
    close(c[0]);
    int statuts_partiels = 0, statuts_faux = 0;
    for (int i = 0; i < ECRIVAINS; i++) {
        int st = 0;
        waitpid(fils[i], &st, 0);
        if (WIFEXITED(st) && WEXITSTATUS(st) >= 100)
            statuts_partiels += WEXITSTATUS(st) - 100;
        else if (!WIFEXITED(st) || WEXITSTATUS(st) != 0)
            statuts_faux++;
    }
    long melanges = 0, sauts = 0;
    int prochain[ECRIVAINS] = { 0 };
    for (long o = 0; o + r <= lus; o += r) {
        unsigned char id = flux[o];
        int ok = id >= 'A' && id < 'A' + ECRIVAINS;
        for (int j = ENTETE; ok && j < r; j++)
            ok = flux[o + j] == id;
        if (!ok) {
            melanges++;
            continue;
        }
        int s = flux[o + 1] | flux[o + 2] << 8 | flux[o + 3] << 16 | flux[o + 4] << 24;
        if (s != prochain[id - 'A'])
            sauts++;
        prochain[id - 'A'] = s + 1;
    }
    free(flux);
    int bon = lus == attendus && melanges == 0 && sauts == 0 && statuts_partiels == 0 && statuts_faux == 0;
    printf("TUBE_ATOMICITE r=%d octets=%ld/%ld melanges=%ld sauts=%ld ecritures_partielles=%d "
           "ecrivains_en_erreur=%d ms=%lld %s\n",
           r, lus, attendus, melanges, sauts, statuts_partiels, statuts_faux, maintenant_ms() - t0,
           bon ? "ok" : "ECHEC");
    if (!bon)
        echecs++;
}

static int pollout(int fd)
{
    struct pollfd p = { .fd = fd, .events = POLLOUT };
    return poll(&p, 1, 0) == 1 && (p.revents & POLLOUT);
}

/* POLLOUT annonce => une ecriture de PIPE_BUF octets passe entiere. */
static void coherence_poll(void)
{
    int c[2];
    if (pipe(c) != 0) {
        printf("TUBE_POLLOUT pipe errno=%d ECHEC\n", errno);
        echecs++;
        return;
    }
    fcntl(c[0], F_SETFL, O_NONBLOCK);
    fcntl(c[1], F_SETFL, O_NONBLOCK);
    static unsigned char tampon[PIPE_BUF];
    memset(tampon, 'P', sizeof tampon);
    long remplis = 0;
    while (remplis < (16L << 20)) {
        ssize_t n = write(c[1], tampon, 1000);
        if (n < 0)
            break;
        remplis += n;
    }
    int plein_errno = errno;
    int annonces = 0, refus_apres_pollout = 0;
    for (int k = 0; k < 12; k++) {
        if (pollout(c[1])) {
            annonces++;
            ssize_t n = write(c[1], tampon, PIPE_BUF);
            if (n != PIPE_BUF)
                refus_apres_pollout++;
        }
        if (read(c[0], tampon, 700) < 0)
            break;
    }
    while (read(c[0], tampon, sizeof tampon) > 0)
        ;
    int vide_pret = pollout(c[1]);
    close(c[0]);
    close(c[1]);
    int bon = plein_errno == EAGAIN && refus_apres_pollout == 0 && vide_pret;
    printf("TUBE_POLLOUT rempli=%ld annonces=%d refus_apres_pollout=%d vide_pret=%d %s\n", remplis, annonces,
           refus_apres_pollout, vide_pret, bon ? "ok" : "ECHEC");
    if (!bon)
        echecs++;
}

int main(int argc, char **argv)
{
    int tours = argc > 1 ? atoi(argv[1]) : 300;
    if (tours < 1) {
        printf("TUBE_ATOMICITE_FAIL parametres\n");
        return 1;
    }
    signal(SIGPIPE, SIG_IGN);
    static const int tailles[] = { 100, 512, PIPE_BUF };
    for (int i = 0; i < 3; i++)
        une_taille(tailles[i], tours);
    coherence_poll();
    if (echecs) {
        printf("TUBE_ATOMICITE_FAIL echecs=%d\n", echecs);
        return 1;
    }
    printf("TUBE_ATOMICITE_OK tailles=3 ecrivains=%d tours=%d\n", ECRIVAINS, tours);
    return 0;
}
