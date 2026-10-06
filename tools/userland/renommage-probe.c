/*
 * Sonde du renommage POSIX : rename, renameat, renameat2.
 *
 * BOUCHAUD_RENOMMAGE_POSIX_V1
 *
 * Rejouee d'abord sur Linux (reference), puis dans Bouchaud par
 * `tools/ci/run_os_primitives.sh`. Chaque cas est un comportement dont un
 * logiciel reel depend sans le dire :
 *
 *   - `WebView::FileDownloader` ecrit `nom.part`, puis le RENOMME sur `nom` ;
 *     l'ancien renommage laissait DEUX entrees `nom` dans le dossier ;
 *   - SQLite et le cache HTTP remplacent des fichiers par renommage ;
 *   - `renameat2(RENAME_NOREPLACE)` est l'ecriture exclusive des magasins ;
 *   - un dossier ne doit pas pouvoir devenir son propre descendant.
 *
 * Sortie : une ligne par cas, puis `RENOMMAGE_OK` ou `RENOMMAGE_ECHEC n=...`.
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifndef SYS_renameat2
#define SYS_renameat2 316
#endif
#ifndef RENAME_NOREPLACE
#define RENAME_NOREPLACE 1
#endif
#ifndef RENAME_EXCHANGE
#define RENAME_EXCHANGE 2
#endif

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-58s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

static int ecrit(const char *chemin, const char *texte)
{
    int fd = open(chemin, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0)
        return -1;
    ssize_t n = write(fd, texte, strlen(texte));
    close(fd);
    return n == (ssize_t)strlen(texte) ? 0 : -1;
}

static int contenu_est(const char *chemin, const char *attendu)
{
    char tampon[128] = { 0 };
    int fd = open(chemin, O_RDONLY);
    if (fd < 0)
        return 0;
    ssize_t n = read(fd, tampon, sizeof(tampon) - 1);
    close(fd);
    return n == (ssize_t)strlen(attendu) && memcmp(tampon, attendu, (size_t)n) == 0;
}

/* Combien d'entrees portent ce nom dans le dossier ? */
static int entrees(const char *dossier, const char *nom)
{
    DIR *d = opendir(dossier);
    if (!d)
        return -1;
    int n = 0;
    struct dirent *e;
    while ((e = readdir(d)) != NULL)
        n += strcmp(e->d_name, nom) == 0;
    closedir(d);
    return n;
}

static long renameat2_brut(int a, const char *de, int b, const char *vers, unsigned drapeaux)
{
    long r = syscall(SYS_renameat2, a, de, b, vers, drapeaux);
    return r < 0 ? -errno : r;
}

int main(void)
{
    char racine[] = "/tmp/renommage-XXXXXX";
    if (!mkdtemp(racine)) {
        printf("RENOMMAGE_ECHEC mkdtemp errno=%d\n", errno);
        return 1;
    }
    if (chdir(racine) != 0) {
        printf("RENOMMAGE_ECHEC chdir errno=%d\n", errno);
        return 1;
    }
    printf("renommage : %s\n", racine);

    /* 1. Le schema de FileDownloader : nom.part -> nom, nom existant. */
    ecrit("rapport.pdf", "ancien");
    ecrit("rapport.pdf.part", "nouveau");
    verifie("rename(part, existant) reussit", rename("rapport.pdf.part", "rapport.pdf") == 0);
    verifie("une seule entree rapport.pdf apres remplacement", entrees(".", "rapport.pdf") == 1);
    verifie("rapport.pdf.part a disparu", entrees(".", "rapport.pdf.part") == 0);
    verifie("le contenu est celui du remplacant", contenu_est("rapport.pdf", "nouveau"));

    /* 2. Deplacement simple, entre dossiers. */
    mkdir("a", 0755);
    mkdir("b", 0755);
    ecrit("a/f", "f");
    verifie("rename(a/f, b/g) reussit", rename("a/f", "b/g") == 0);
    verifie("b/g existe, a/f non", entrees("b", "g") == 1 && entrees("a", "f") == 0);

    /* 3. Meme fichier : succes, rien ne change. */
    verifie("rename(b/g, b/g) reussit sans rien changer", rename("b/g", "b/g") == 0 && entrees("b", "g") == 1);

    /* 4. Erreurs de type. */
    mkdir("plein", 0755);
    ecrit("plein/x", "x");
    mkdir("vide", 0755);
    errno = 0;
    verifie("fichier -> dossier : EISDIR", rename("b/g", "vide") != 0 && errno == EISDIR);
    errno = 0;
    verifie("dossier -> fichier : ENOTDIR", rename("vide", "b/g") != 0 && errno == ENOTDIR);
    errno = 0;
    verifie("dossier -> dossier non vide : ENOTEMPTY ou EEXIST",
        rename("vide", "plein") != 0 && (errno == ENOTEMPTY || errno == EEXIST));
    mkdir("vide2", 0755);
    verifie("dossier -> dossier vide : remplace", rename("vide2", "vide") == 0 && entrees(".", "vide") == 1 && entrees(".", "vide2") == 0);

    /* 5. Un dossier dans son propre sous-arbre. */
    mkdir("p", 0755);
    mkdir("p/q", 0755);
    errno = 0;
    verifie("rename(p, p/q/p) : EINVAL", rename("p", "p/q/p") != 0 && errno == EINVAL);
    verifie("l'arbre p/q est intact", entrees("p", "q") == 1);

    /* 6. Source absente, parent absent. */
    errno = 0;
    verifie("source absente : ENOENT", rename("absent", "x") != 0 && errno == ENOENT);
    errno = 0;
    verifie("parent cible absent : ENOENT", rename("b/g", "nulle/part") != 0 && errno == ENOENT);

    /* 7. renameat, relatif a des descripteurs de dossier. */
    int da = open("a", O_RDONLY | O_DIRECTORY);
    int db = open("b", O_RDONLY | O_DIRECTORY);
    verifie("renameat(db, g, da, h) reussit", da >= 0 && db >= 0 && renameat(db, "g", da, "h") == 0);
    verifie("a/h existe, b/g non", entrees("a", "h") == 1 && entrees("b", "g") == 0);

    /* 8. renameat2 RENAME_NOREPLACE. */
    ecrit("a/k", "k");
    verifie("NOREPLACE sur cible existante : EEXIST", renameat2_brut(da, "h", da, "k", RENAME_NOREPLACE) == -EEXIST);
    verifie("NOREPLACE sur cible libre : reussit", renameat2_brut(da, "h", da, "h2", RENAME_NOREPLACE) == 0);

    /* 9. renameat2 RENAME_EXCHANGE. */
    ecrit("a/u", "u");
    ecrit("b/v", "v");
    verifie("EXCHANGE reussit", renameat2_brut(da, "u", db, "v", RENAME_EXCHANGE) == 0);
    verifie("les contenus sont echanges", contenu_est("a/u", "v") && contenu_est("b/v", "u"));
    verifie("EXCHANGE sans cible : ENOENT", renameat2_brut(da, "u", db, "absent", RENAME_EXCHANGE) == -ENOENT);
    verifie("NOREPLACE|EXCHANGE : EINVAL",
        renameat2_brut(da, "u", db, "v", RENAME_NOREPLACE | RENAME_EXCHANGE) == -EINVAL);
    verifie("drapeau inconnu : EINVAL", renameat2_brut(da, "u", db, "w", 0x80) == -EINVAL);
    if (da >= 0)
        close(da);
    if (db >= 0)
        close(db);

    if (echecs == 0)
        printf("RENOMMAGE_OK cas=24\n");
    else
        printf("RENOMMAGE_ECHEC n=%d\n", echecs);
    return echecs == 0 ? 0 : 1;
}
