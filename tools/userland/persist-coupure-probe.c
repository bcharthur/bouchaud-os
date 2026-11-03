/*
 * Sonde : coupure d'alimentation pendant le cycle d'ecriture atomique d'un
 * fichier utilisateur (BOUCHAUD_PERSIST_COUPURE_V1).
 *
 * Cycle POSIX classique, tel qu'une application le fait :
 *
 *   ecrire etat.tmp  ->  fsync(etat.tmp)  ->  rename(etat.tmp, etat)
 *   ->  fsync(repertoire)  ->  [acquitte : COUPURE_ACQUITTE version=N]
 *
 * L'hote (tools/ci/run_coupure_persistance.sh) tue QEMU par SIGKILL a un
 * instant arbitraire, puis redemarre sur le MEME disque. Au demarrage, la
 * sonde relit /persist/coupure/etat et publie :
 *
 *   COUPURE_RELU version=N coherent=1|0 [raison=...]
 *
 * Invariants verifies par l'hote : le fichier est TOUJOURS coherent (une
 * version entiere, l'ancienne ou la nouvelle, jamais un melange), et sa
 * version n'est jamais inferieure a la derniere version ACQUITTEE avant la
 * coupure (un fsync acquitte est durable).
 *
 * Format : 64 Kio. Chaque ligne de 64 octets porte la version et l'indice
 * de la ligne ; la derniere porte une somme de controle de tout le reste.
 * Un melange de deux versions, un fichier tronque ou une ligne d'une autre
 * version se voient tous.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#define DOSSIER "/persist/coupure"
#define ETAT    DOSSIER "/etat"
#define TMP     DOSSIER "/etat.tmp"
#define TAILLE  65536
#define LIGNE   64
#define LIGNES  (TAILLE / LIGNE)

static char tampon[TAILLE];

static unsigned long somme(const char *p, size_t n)
{
    unsigned long h = 1469598103934665603UL;
    for (size_t i = 0; i < n; i++) { h ^= (unsigned char)p[i]; h *= 1099511628211UL; }
    return h;
}

static void construit(unsigned long version)
{
    for (int i = 0; i < LIGNES - 1; i++) {
        char *l = tampon + (size_t)i * LIGNE;
        int n = snprintf(l, LIGNE, "v=%020lu l=%05d ", version, i);
        memset(l + n, 'a' + (int)(version % 26), LIGNE - n - 1);
        l[LIGNE - 1] = '\n';
    }
    char *fin = tampon + (size_t)(LIGNES - 1) * LIGNE;
    int n = snprintf(fin, LIGNE, "v=%020lu somme=%016lx ", version, somme(tampon, TAILLE - LIGNE));
    memset(fin + n, '.', LIGNE - n - 1);
    fin[LIGNE - 1] = '\n';
}

/* 1 coherent, 0 incoherent ; *version et *raison renseignes. */
static int verifie(unsigned long *version, const char **raison)
{
    int fd = open(ETAT, O_RDONLY);
    if (fd < 0) { *version = 0; *raison = errno == ENOENT ? "absent" : "ouverture"; return errno == ENOENT; }
    ssize_t lu = 0, n;
    while (lu < TAILLE && (n = read(fd, tampon + lu, TAILLE - lu)) > 0) lu += n;
    char octet;
    int surplus = read(fd, &octet, 1) > 0;
    close(fd);
    if (lu != TAILLE || surplus) { *raison = "taille"; return 0; }
    if (sscanf(tampon, "v=%020lu", version) != 1) { *raison = "entete"; return 0; }
    for (int i = 0; i < LIGNES - 1; i++) {
        unsigned long v; int idx;
        if (sscanf(tampon + (size_t)i * LIGNE, "v=%020lu l=%05d", &v, &idx) != 2 || v != *version || idx != i) {
            *raison = "melange"; return 0;
        }
    }
    unsigned long v, s;
    if (sscanf(tampon + (size_t)(LIGNES - 1) * LIGNE, "v=%020lu somme=%016lx", &v, &s) != 2
        || v != *version || s != somme(tampon, TAILLE - LIGNE)) {
        *raison = "somme"; return 0;
    }
    *raison = "ok";
    return 1;
}

static int ecrit_tout(int fd, const char *p, size_t n)
{
    while (n) {
        ssize_t w = write(fd, p, n);
        if (w <= 0) return -1;
        p += w; n -= (size_t)w;
    }
    return 0;
}

int main(int argc, char **argv)
{
    unsigned long version;
    const char *raison;
    int cycles = argc > 1 ? atoi(argv[1]) : 0;   /* 0: forever (cut by the host) */

    mkdir("/persist", 0755);
    mkdir(DOSSIER, 0755);
    unlink(TMP);   /* a leftover temp file is never the state */
    int coherent = verifie(&version, &raison);
    printf("COUPURE_RELU version=%lu coherent=%d raison=%s\n", version, coherent, raison);
    fflush(stdout);
    if (!coherent) return 1;

    for (int c = 0; cycles == 0 || c < cycles; c++) {
        unsigned long suivante = version + 1;
        construit(suivante);
        int fd = open(TMP, O_WRONLY | O_CREAT | O_TRUNC, 0644);
        if (fd < 0 || ecrit_tout(fd, tampon, TAILLE) != 0 || fsync(fd) != 0) {
            printf("COUPURE_ECHEC etape=tmp errno=%d\n", errno); fflush(stdout); return 2;
        }
        close(fd);
        if (rename(TMP, ETAT) != 0) { printf("COUPURE_ECHEC etape=rename errno=%d\n", errno); fflush(stdout); return 3; }
        int d = open(DOSSIER, O_RDONLY | O_DIRECTORY);
        if (d < 0 || fsync(d) != 0) { printf("COUPURE_ECHEC etape=fsync_dir errno=%d\n", errno); fflush(stdout); return 4; }
        close(d);
        version = suivante;
        printf("COUPURE_ACQUITTE version=%lu\n", version);
        fflush(stdout);
    }
    return 0;
}
