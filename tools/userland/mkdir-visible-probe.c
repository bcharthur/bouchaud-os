/*
 * Sonde : `mkdir` d'un chemin existant rend EEXIST avant le controle de
 * creation -- s'il est visible de l'appelant.
 *
 * BOUCHAUD_MKDIR_EEXIST_AVANT_EACCES_V1
 *
 * `Core::Directory::ensure_directory` (LibCore) cree chaque ancetre d'un
 * chemin en ignorant EEXIST. RequestServer recevait EACCES sur
 * `mkdir("/persist")` et n'a jamais cree son cache disque (smoke 35b64893) :
 *
 *   [SECURITY-DENY] pid=16 op=fs-create path=/persist reason=outside-sandbox
 *   Unable to create disk cache: mkdir: Permission denied (errno=13)
 *
 * Deux usages :
 *   mkdir-visible-probe prepare   hors bac a sable : cree /persist/Downloads/x
 *   RequestServer                 la MEME sonde, copiee sous ce nom : le noyau
 *                                 lui donne le role BrowserNetwork
 *                                 (`security/profile.rs`), confine.
 *
 * BOUCHAUD_FCHOWN_SANS_EFFET_V1 : la meme sonde confinee verifie aussi qu'un
 * `fchown` qui ne change rien (ce que SQLite fait sur ses `-wal`/`-shm` quand
 * euid=0) reussit pour le proprietaire, et qu'un vrai changement reste refuse.
 *
 * Sortie : `MKDIR_VISIBLE_OK` ou `MKDIR_VISIBLE_ECHEC n=...`.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <unistd.h>

static int echecs = 0;

static void verifie(const char *quoi, int condition, int erreur)
{
    printf("  %-60s %s (errno=%d)\n", quoi, condition ? "ok" : "ECHEC", erreur);
    if (!condition)
        echecs++;
}

/* mkdir, et l'errno qu'il laisse. */
static int essaie(const char *chemin, int *erreur)
{
    errno = 0;
    int r = mkdir(chemin, 0700);
    *erreur = r == 0 ? 0 : errno;
    return r;
}

/* Ce que fait `Core::Directory::ensure_directory` : chaque ancetre, EEXIST
 * ignore, toute autre erreur fatale. */
static int ensure_directory(const char *chemin, int *erreur)
{
    char partiel[256];
    size_t n = strlen(chemin);
    if (n >= sizeof partiel)
        return -1;
    for (size_t i = 1; i <= n; i++) {
        if (chemin[i] != '/' && chemin[i] != '\0')
            continue;
        memcpy(partiel, chemin, i);
        partiel[i] = '\0';
        if (essaie(partiel, erreur) != 0 && *erreur != EEXIST) {
            printf("    ensure_directory : %s -> errno=%d\n", partiel, *erreur);
            return -1;
        }
    }
    *erreur = 0;
    return 0;
}

int main(int argc, char **argv)
{
    int e = 0;
    if (argc > 1 && strcmp(argv[1], "prepare") == 0) {
        // Ce que fait le processus navigateur (non sandboxe) AVANT de lancer
        // RequestServer (`ui-bouchaud/main.cpp`) : la racine du profil et
        // celle du cache existent. BOUCHAUD_PROFIL_PAR_ROLE_V1 : RequestServer
        // ne possede plus que le cache ; creer `/persist/ladybird` lui-meme
        // lui est refuse, et il n'en a pas besoin.
        mkdir("/persist/ladybird", 0700);
        mkdir("/persist/ladybird/cache", 0700);
        mkdir("/persist/Downloads", 0755);
        int r = essaie("/persist/Downloads/x", &e);
        printf("mkdir-visible-probe prepare : /persist/Downloads/x r=%d errno=%d\n", r, e);
        return 0;
    }

    int confine = prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0);
    printf("mkdir-visible-probe : argv0=%s no_new_privs=%d\n", argv[0], confine);
    verifie("la sonde tourne confinee (no_new_privs=1)", confine == 1, 0);

    int r = essaie("/persist", &e);
    verifie("mkdir(/persist), visible et existant : EEXIST", r != 0 && e == EEXIST, e);

    r = ensure_directory("/persist/ladybird/cache/Ladybird/Profiles/default/Cache", &e);
    verifie("ensure_directory(cache du profil) reussit", r == 0, e);
    struct stat etat;
    verifie("le dossier du cache existe", stat("/persist/ladybird/cache/Ladybird/Profiles/default/Cache", &etat) == 0
        && S_ISDIR(etat.st_mode), errno);

    /* fchown : sans effet -> accepte ; vrai changement -> EPERM. */
    int fd = open("/persist/ladybird/cache/Ladybird/Profiles/default/Cache/index.db-wal",
        O_WRONLY | O_CREAT | O_TRUNC, 0600);
    verifie("un fichier -wal se cree dans le profil", fd >= 0, errno);
    if (fd >= 0) {
        struct stat f;
        fstat(fd, &f);
        errno = 0;
        int a = fchown(fd, (uid_t)-1, (gid_t)-1);
        verifie("fchown(fd, -1, -1) sans effet : accepte", a == 0, errno);
        errno = 0;
        int b = fchown(fd, f.st_uid, f.st_gid);
        verifie("fchown(fd, uid, gid actuels) sans effet : accepte", b == 0, errno);
        errno = 0;
        int c = fchown(fd, f.st_uid + 1000, (gid_t)-1);
        int ec = errno;
        verifie("fchown vers un autre proprietaire : EPERM", c != 0 && ec == EPERM, ec);
        struct stat apres;
        fstat(fd, &apres);
        verifie("le proprietaire n'a pas change", apres.st_uid == f.st_uid && apres.st_gid == f.st_gid, 0);
        close(fd);
    }

    r = essaie("/persist/Downloads/x", &e);
    verifie("mkdir(/persist/Downloads/x), existant mais INVISIBLE : EACCES", r != 0 && e == EACCES, e);
    r = essaie("/persist/Downloads/neuf", &e);
    verifie("mkdir(/persist/Downloads/neuf), absent et interdit : EACCES", r != 0 && e == EACCES, e);
    r = essaie("/dossier-neuf-a-la-racine", &e);
    verifie("mkdir(/dossier-neuf-a-la-racine) : EACCES", r != 0 && e == EACCES, e);
    r = essaie("/persist/ladybird/data", &e);
    verifie("mkdir(/persist/ladybird/data) : EACCES (profil hors cache)", r != 0 && e == EACCES, e);
    r = essaie("/persist/ladybird-chrome", &e);
    verifie("mkdir(/persist/ladybird-chrome) : EACCES (pas au role reseau)", r != 0 && e == EACCES, e);

    if (echecs == 0)
        printf("MKDIR_VISIBLE_OK\n");
    else
        printf("MKDIR_VISIBLE_ECHEC n=%d\n", echecs);
    return echecs == 0 ? 0 : 1;
}
