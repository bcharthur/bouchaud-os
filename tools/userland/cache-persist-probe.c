/*
 * Sonde : un cache qui deborde ne doit pas emporter la persistance.
 *
 * BOUCHAUD_PERSIST_CACHE_JETABLE_V1
 *
 * Deux passages, sur la MEME image, separes par un redemarrage ; la sonde
 * reconnait seule le sien (comme `persist-probe.c`).
 *
 * Passage 1 :
 *   - `statfs("/persist")` annonce la ZONE (au plus 64 Mio), pas la memoire
 *     vive : c'est sur ce chiffre que Ladybird dimensionne son cache HTTP ;
 *   - on depose ce qu'un navigateur ne doit jamais perdre (une base de
 *     temoins, des reglages), un GROS cache etiquete `CACHEDIR.TAG` qui depasse
 *     a lui seul les 2048 entrees de la zone, et un PETIT cache etiquete ;
 *   - `fsync` sur un fichier de `/persist` doit REUSSIR. Avant la correction,
 *     il rendait EIO : la zone refusait tout des qu'elle debordait.
 *
 * Passage 2 :
 *   - les temoins et les reglages sont intacts ;
 *   - le gros cache a disparu EN ENTIER (aucun fichier isole, pas meme son
 *     etiquette) ; le petit, qui tenait, est intact.
 *
 * Sortie : `CACHE_PERSIST_PASSAGE1_OK`, puis `CACHE_PERSIST_OK` au second
 * demarrage ; `CACHE_PERSIST_ECHEC n=...` sinon.
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/statfs.h>
#include <unistd.h>

#define RACINE "/persist/cache-jetable"
#define TEMOINS RACINE "/data/temoins.db"
#define REGLAGES RACINE "/config/reglages.json"
#define GROS RACINE "/cache"
#define PETIT RACINE "/petit-cache"
#define FICHIERS_GROS 2100
#define FICHIERS_PETIT 3

static const char SIGNATURE[] = "Signature: 8a477f597d28d172789f06886806bc55\n# sonde\n";
static const char CONTENU_TEMOINS[] = "SQLite format 3\0 session=depuis-le-disque";
static const char CONTENU_REGLAGES[] = "{\"diskCache\":{\"maxSize\":33554432}}";

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-62s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

static int ecrit(const char *chemin, const void *octets, size_t taille)
{
    int fd = open(chemin, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd < 0)
        return -1;
    ssize_t n = write(fd, octets, taille);
    close(fd);
    return n == (ssize_t)taille ? 0 : -1;
}

static int contenu_est(const char *chemin, const void *attendu, size_t taille)
{
    char tampon[256];
    int fd = open(chemin, O_RDONLY);
    if (fd < 0)
        return 0;
    ssize_t n = read(fd, tampon, sizeof tampon);
    close(fd);
    return n == (ssize_t)taille && memcmp(tampon, attendu, taille) == 0;
}

static int entrees(const char *dossier)
{
    DIR *d = opendir(dossier);
    if (!d)
        return -1;
    int n = 0;
    struct dirent *e;
    while ((e = readdir(d)) != NULL)
        n += strcmp(e->d_name, ".") != 0 && strcmp(e->d_name, "..") != 0;
    closedir(d);
    return n;
}

static int passage1(void)
{
    printf("cache-persist-probe : passage 1 -- depot\n");

    struct statfs zone, ram;
    int a = statfs("/persist", &zone);
    int b = statfs("/tmp", &ram);
    unsigned long long total_zone = (unsigned long long)zone.f_blocks * (unsigned long long)zone.f_frsize;
    unsigned long long libre_zone = (unsigned long long)zone.f_bavail * (unsigned long long)zone.f_frsize;
    unsigned long long total_ram = (unsigned long long)ram.f_blocks * (unsigned long long)ram.f_frsize;
    printf("  statfs /persist total=%llu libre=%llu fichiers=%llu libres=%llu ; /tmp total=%llu\n",
        total_zone, libre_zone, (unsigned long long)zone.f_files, (unsigned long long)zone.f_ffree, total_ram);
    verifie("statfs(/persist) et statfs(/tmp) reussissent", a == 0 && b == 0);
    verifie("/persist annonce la zone : 32 Mio < total <= 64 Mio",
        total_zone > 32ull << 20 && total_zone <= 64ull << 20);
    verifie("/persist annonce moins que la RAM", total_zone < total_ram);
    verifie("/persist annonce les 2048 entrees de la table", zone.f_files == 2048);
    verifie("l'espace libre de la zone ne depasse pas son total", libre_zone <= total_zone);

    mkdir(RACINE, 0700);
    mkdir(RACINE "/data", 0700);
    mkdir(RACINE "/config", 0700);
    mkdir(GROS, 0700);
    mkdir(GROS "/Ladybird", 0700);
    mkdir(PETIT, 0700);
    verifie("la base de temoins s'ecrit", ecrit(TEMOINS, CONTENU_TEMOINS, sizeof CONTENU_TEMOINS) == 0);
    verifie("les reglages s'ecrivent", ecrit(REGLAGES, CONTENU_REGLAGES, sizeof CONTENU_REGLAGES - 1) == 0);
    verifie("le gros cache est etiquete", ecrit(GROS "/CACHEDIR.TAG", SIGNATURE, sizeof SIGNATURE - 1) == 0);
    verifie("le petit cache est etiquete", ecrit(PETIT "/CACHEDIR.TAG", SIGNATURE, sizeof SIGNATURE - 1) == 0);

    int ecrits = 0;
    char chemin[128], corps[64];
    for (int i = 0; i < FICHIERS_GROS; i++) {
        snprintf(chemin, sizeof chemin, GROS "/Ladybird/%05d", i);
        int n = snprintf(corps, sizeof corps, "HTTP/1.1 200 OK reponse %d", i);
        ecrits += ecrit(chemin, corps, (size_t)n) == 0;
    }
    printf("  gros cache : %d/%d fichiers\n", ecrits, FICHIERS_GROS);
    verifie("le gros cache depasse a lui seul les 2048 entrees", ecrits == FICHIERS_GROS);
    for (int i = 0; i < FICHIERS_PETIT; i++) {
        snprintf(chemin, sizeof chemin, PETIT "/r%d", i);
        int n = snprintf(corps, sizeof corps, "petit %d", i);
        ecrit(chemin, corps, (size_t)n);
    }

    // L'appel qui compte : `fsync` sur un fichier persistant ecrit la zone.
    int fd = open(TEMOINS, O_RDONLY);
    errno = 0;
    int r = fd >= 0 ? fsync(fd) : -1;
    int erreur = errno;
    if (fd >= 0)
        close(fd);
    printf("  fsync(temoins) = %d errno=%d\n", r, erreur);
    verifie("fsync reussit malgre le cache qui deborde (pas d'EIO)", r == 0);

    if (echecs == 0)
        printf("CACHE_PERSIST_PASSAGE1_OK\n");
    else
        printf("CACHE_PERSIST_ECHEC passage=1 n=%d\n", echecs);
    return echecs == 0 ? 0 : 1;
}

static int passage2(void)
{
    printf("cache-persist-probe : passage 2 -- relecture apres redemarrage\n");
    verifie("la base de temoins a survecu, intacte", contenu_est(TEMOINS, CONTENU_TEMOINS, sizeof CONTENU_TEMOINS));
    verifie("les reglages ont survecu, intacts", contenu_est(REGLAGES, CONTENU_REGLAGES, sizeof CONTENU_REGLAGES - 1));

    int gros = entrees(GROS "/Ladybird");
    struct stat etat;
    int etiquette = stat(GROS "/CACHEDIR.TAG", &etat) == 0;
    printf("  gros cache apres redemarrage : %d fichier(s), etiquette=%d\n", gros, etiquette);
    verifie("le gros cache est ecarte EN ENTIER (aucun fichier isole)", gros <= 0 && !etiquette);

    int petit = entrees(PETIT);
    printf("  petit cache apres redemarrage : %d entree(s)\n", petit);
    verifie("le petit cache, qui tenait, est intact (3 + etiquette)", petit == FICHIERS_PETIT + 1);
    verifie("et son contenu aussi", contenu_est(PETIT "/r1", "petit 1", 7));

    if (echecs == 0)
        printf("CACHE_PERSIST_OK\n");
    else
        printf("CACHE_PERSIST_ECHEC passage=2 n=%d\n", echecs);
    return echecs == 0 ? 0 : 1;
}

int main(void)
{
    struct stat etat;
    if (stat(TEMOINS, &etat) == 0 && etat.st_size > 0)
        return passage2();
    return passage1();
}
