/*
 * Sonde du protocole de memoire partagee de SQLite en mode WAL.
 *
 * `LibDatabase/Database.cpp` met la base en `journal_mode = WAL` des son
 * ouverture. Le WAL n'est pas un detail de performance : il change les
 * primitives que SQLite exige de l'OS. En plus du fichier de base, il ouvre un
 * fichier `<base>-shm`, l'agrandit, le projette en `MAP_SHARED`, et s'en sert
 * comme d'une memoire partagee entre TOUS les processus qui ouvrent la base --
 * avec des verrous d'enregistrement poses dans une plage precise de ce fichier
 * (128 octets a partir de l'offset 120).
 *
 * Trois exigences en decoulent, et aucune n'etait couverte :
 *
 *   1. `ftruncate` doit agrandir un fichier ordinaire ;
 *   2. deux processus qui ouvrent LE MEME CHEMIN et le projettent en
 *      MAP_SHARED doivent voir les ecritures l'un de l'autre -- pas une copie
 *      chacun. `shm-probe.c` ne verifie que le cas d'un descripteur herite par
 *      `fork`, ce qui est une autre situation ;
 *   3. les verrous d'enregistrement doivent fonctionner sur ce fichier-la.
 *
 * Si l'une manque, SQLite rend SQLITE_IOERR, et l'application n'a plus que
 * « disk I/O error » a dire -- exactement ce que le BrowserHost a rapporte au
 * run 32424806818.
 *
 *   musl-gcc -O2 -static-pie wal-probe.c -o wal-probe
 *   (ou via ./build.sh musl)
 */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

/* Taille d'une region shm SQLite (SQLITE_DEFAULT_WAL_SHM_SIZE). */
#define TAILLE_SHM 32768

/* Plage de verrous du WAL : SQLITE_SHM_BASE = 120, SQLITE_SHM_NLOCK = 8. */
#define SHM_BASE 120
#define SHM_NLOCK 8

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-52s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

static const char *repertoire(void)
{
    /* La ou le navigateur met sa base : sous la zone persistante si elle
     * existe, sinon /tmp, pour que la sonde reste utilisable partout. */
    struct stat st;
    if (stat("/persist", &st) == 0 && S_ISDIR(st.st_mode))
        return "/persist";
    return "/tmp";
}

int main(void)
{
    printf("Sonde WAL SQLite — fichier partage, projection, verrous\n\n");

    char chemin_db[256], chemin_shm[256];
    snprintf(chemin_db, sizeof(chemin_db), "%s/wal-probe.db", repertoire());
    snprintf(chemin_shm, sizeof(chemin_shm), "%s/wal-probe.db-shm", repertoire());
    printf("  repertoire de travail : %s\n\n", repertoire());

    unlink(chemin_db);
    unlink(chemin_shm);

    /* --- le fichier de base ------------------------------------------- */
    printf("[fichier de base]\n");
    int db = open(chemin_db, O_RDWR | O_CREAT, 0644);
    verifie("open() du fichier de base", db >= 0);
    if (db < 0) { printf("WAL_PROBE_FAIL echecs=%d\n", ++echecs); return 1; }

    verifie("write() dans le fichier de base", write(db, "SQLite format 3", 15) == 15);
    /* SQLite appelle fsync a chaque validation. Sous /persist, cela declenche
     * l'ecriture de la zone persistante : c'est LE chemin qui rend EIO. */
    int r = fsync(db);
    printf("      fsync() = %d%s%s\n", r, r == 0 ? "" : " errno=", r == 0 ? "" : strerror(errno));
    verifie("fsync() du fichier de base", r == 0);

    /* --- le fichier -shm ---------------------------------------------- */
    printf("\n[fichier -shm]\n");
    int shm = open(chemin_shm, O_RDWR | O_CREAT, 0644);
    verifie("open() du fichier -shm", shm >= 0);
    if (shm < 0) { printf("WAL_PROBE_FAIL echecs=%d\n", ++echecs); return 1; }

    verifie("ftruncate() agrandit a 32768", ftruncate(shm, TAILLE_SHM) == 0);
    struct stat st;
    verifie("fstat() confirme la nouvelle taille",
        fstat(shm, &st) == 0 && st.st_size == TAILLE_SHM);

    char *zone = mmap(NULL, TAILLE_SHM, PROT_READ | PROT_WRITE, MAP_SHARED, shm, 0);
    verifie("mmap(MAP_SHARED) du -shm", zone != MAP_FAILED);
    if (zone == MAP_FAILED) {
        printf("      mmap : %s\n", strerror(errno));
        printf("WAL_PROBE_FAIL echecs=%d\n", ++echecs);
        return 1;
    }

    memcpy(zone, "PARENT", 6);
    verifie("relecture de sa propre ecriture", memcmp(zone, "PARENT", 6) == 0);

    /* --- partage entre DEUX processus par le MEME CHEMIN ---------------- */
    printf("\n[partage entre processus par le chemin]\n");

    int tube[2];
    if (pipe(tube) != 0) { printf("WAL_PROBE_FAIL pipe\n"); return 1; }

    pid_t enfant = fork();
    if (enfant < 0) { printf("WAL_PROBE_FAIL fork\n"); return 1; }

    if (enfant == 0) {
        close(tube[0]);
        int resultat[2] = { 0, 0 };
        /* Le fils OUVRE LE FICHIER LUI-MEME : il n'herite pas du descripteur,
         * exactement comme un second processus SQLite. */
        int fd = open(chemin_shm, O_RDWR);
        if (fd >= 0) {
            char *vue = mmap(NULL, TAILLE_SHM, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
            if (vue != MAP_FAILED) {
                resultat[0] = memcmp(vue, "PARENT", 6) == 0;
                memcpy(vue + 16, "ENFANT", 6);
                msync(vue, TAILLE_SHM, MS_SYNC);
                resultat[1] = 1;
                munmap(vue, TAILLE_SHM);
            }
            close(fd);
        }
        ssize_t ignore = write(tube[1], resultat, sizeof(resultat));
        (void)ignore;
        close(tube[1]);
        _exit(0);
    }

    close(tube[1]);
    int resultat[2] = { -1, -1 };
    ssize_t lus = read(tube[0], resultat, sizeof(resultat));
    close(tube[0]);
    int statut = 0;
    waitpid(enfant, &statut, 0);

    verifie("l'enfant a rendu son verdict", lus == (ssize_t)sizeof(resultat));
    verifie("l'enfant voit l'ecriture du parent", resultat[0] == 1);
    verifie("l'enfant a pu ecrire dans sa projection", resultat[1] == 1);
    printf("      octets vus par le parent a l'offset 16 : \"%.6s\"\n", zone + 16);
    verifie("le parent voit l'ecriture de l'enfant", memcmp(zone + 16, "ENFANT", 6) == 0);

    /* --- verrous sur la plage du WAL ------------------------------------ */
    printf("\n[verrous sur la plage WAL]\n");
    struct flock v;
    memset(&v, 0, sizeof(v));
    v.l_type = F_WRLCK;
    v.l_whence = SEEK_SET;
    v.l_start = SHM_BASE;
    v.l_len = SHM_NLOCK;
    verifie("F_SETLK sur [120,128) du -shm", fcntl(shm, F_SETLK, &v) == 0);

    memset(&v, 0, sizeof(v));
    v.l_type = F_WRLCK;
    v.l_whence = SEEK_SET;
    v.l_start = SHM_BASE;
    v.l_len = SHM_NLOCK;
    verifie("F_GETLK par le proprietaire rend F_UNLCK",
        fcntl(shm, F_GETLK, &v) == 0 && v.l_type == F_UNLCK);

    memset(&v, 0, sizeof(v));
    v.l_type = F_UNLCK;
    v.l_whence = SEEK_SET;
    v.l_start = SHM_BASE;
    v.l_len = SHM_NLOCK;
    verifie("F_SETLK F_UNLCK sur la meme plage", fcntl(shm, F_SETLK, &v) == 0);

    /* --- synchronisation finale ----------------------------------------- */
    printf("\n[synchronisation]\n");
    verifie("msync() de la projection", msync(zone, TAILLE_SHM, MS_SYNC) == 0);
    r = fsync(shm);
    printf("      fsync(-shm) = %d%s\n", r, r == 0 ? "" : strerror(errno));
    verifie("fsync() du -shm", r == 0);

    munmap(zone, TAILLE_SHM);
    close(shm);
    close(db);
    unlink(chemin_shm);
    unlink(chemin_db);

    printf("\nRESULTAT : %d verification(s) en echec\n", echecs);
    printf("WAL_PROBE_%s echecs=%d\n", echecs == 0 ? "OK" : "FAIL", echecs);
    return echecs == 0 ? 0 : 1;
}
