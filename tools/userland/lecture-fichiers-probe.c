// Sonde ring 3 : un fichier ordinaire se lit EXACTEMENT, par toutes les voies.
//
// BOUCHAUD_COPIE_BORNEE_V1 / BOUCHAUD_LECTURE_EXACTE_V1 -- trois defauts du
// noyau trouves en faisant tourner OpenJDK 21 :
//
//   1. un `read()` de 140 Mio faisait PANIQUER le noyau : il allouait la
//      taille demandee en une seule fois sur son tas, sans bloc contigu assez
//      grand ;
//   2. `read`/`pread` rendaient des lectures COURTES au milieu du fichier, a
//      chaque frontiere d'etendue du disque paresseux (64 Kio) : la JVM lisait
//      ses classes tronquees (ClassFormatError) ;
//   3. cote `write`, meme allocation sans borne que 1 : un seul `write` de
//      150 Mio paniquait avant que le RAMFS n'ait pu le refuser.
//
// La sonde compare `read` (un seul appel sur tout le fichier), `pread` a 4 000
// positions pseudo-aleatoires et `lseek`+`read` a 2 000 positions, a la
// projection `mmap` du meme fichier, octet par octet. Elle ecrit aussi un
// gros fichier en un seul `write` et le relit.
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/stat.h>

static uint64_t fnv(const unsigned char *p, size_t n) {
    uint64_t h = 1469598103934665603ULL;
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 1099511628211ULL; }
    return h;
}
static uint64_t x = 88172645463325252ULL;
static uint64_t alea(void) { x ^= x << 13; x ^= x >> 7; x ^= x << 17; return x; }

int main(int argc, char **argv) {
    const char *f = argv[1];
    int fd = open(f, O_RDONLY);
    struct stat st;
    if (fd < 0 || fstat(fd, &st) != 0) { puts("LECTURE_ECHEC open"); return 1; }
    size_t n = st.st_size;

    // 1. Un seul read() sur tout le fichier : le noyau ne doit pas paniquer,
    //    et doit tout rendre.
    unsigned char *tout = malloc(n);
    ssize_t r = read(fd, tout, n);
    unsigned char *m = mmap(0, n, PROT_READ, MAP_PRIVATE, fd, 0);
    if (m == MAP_FAILED) { puts("LECTURE_ECHEC mmap"); return 1; }
    printf("LECTURE_GROS_READ taille=%zu rendu=%zd egal_mmap=%d empreinte=%016llx\n",
           n, r, r == (ssize_t)n && memcmp(tout, m, n) == 0, (unsigned long long)fnv(m, n));

    // 2. pread a des positions et longueurs pseudo-aleatoires.
    unsigned char *b = malloc(1 << 20);
    int pread_faux = 0;
    for (int i = 0; i < 4000; i++) {
        size_t off = alea() % n, len = 1 + alea() % ((i & 7) ? 4096 : (1 << 20));
        if (off + len > n) len = n - off;
        ssize_t got = pread(fd, b, len, off);
        if (got != (ssize_t)len || memcmp(b, m + off, len) != 0) {
            if (pread_faux < 4) printf("LECTURE_PREAD_FAUX off=%zu len=%zu rendu=%zd\n", off, len, got);
            pread_faux++;
        }
    }
    // 3. lseek + read, en petits morceaux comme un lecteur d'archive.
    int read_faux = 0;
    for (int i = 0; i < 2000; i++) {
        size_t off = alea() % n, len = 1 + alea() % 8192;
        if (off + len > n) len = n - off;
        lseek(fd, off, SEEK_SET);
        ssize_t got = read(fd, b, len);
        if (got != (ssize_t)len || memcmp(b, m + off, len) != 0) {
            if (read_faux < 4) printf("LECTURE_READ_FAUX off=%zu len=%zu rendu=%zd\n", off, len, got);
            read_faux++;
        }
    }
    // 4. Un seul write() de tout le contenu, puis relecture.
    int out = open("/tmp/lecture-copie", O_CREAT | O_TRUNC | O_RDWR, 0644);
    ssize_t w = write(out, m, n);
    lseek(out, 0, SEEK_SET);
    memset(tout, 0, n);
    ssize_t r2 = read(out, tout, n);
    // Un fichier RAMFS est plafonne (64 Mio aujourd'hui) : une ecriture
    // PARTIELLE est legitime. Ce qui compte : pas de panique, et ce qui a ete
    // ecrit se relit a l'identique.
    int copie_ok = w > 0 && r2 == w && memcmp(tout, m, (size_t)w) == 0;
    printf("LECTURE_GROS_WRITE demande=%zu ecrit=%zd relu=%zd egal=%d\n", n, w, r2, copie_ok);

    printf("LECTURE_FIN pread_faux=%d read_faux=%d\n", pread_faux, read_faux);
    return 0;
}
