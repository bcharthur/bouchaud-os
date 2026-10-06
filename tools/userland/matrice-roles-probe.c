/*
 * Matrice de securite des roles du navigateur, par de VRAIS appels systeme.
 *
 * BOUCHAUD_MATRICE_ROLES_V1 (convergence P2)
 *
 * Le noyau classe un processus d'apres le nom de son image
 * (`security/profile.rs`) : copiee sous /usr/libexec/ladybird/WebContent, la
 * sonde EST un WebContent pour le noyau, confine comme lui. Chaque operation
 * est tentee pour de bon ; le resultat (succes, ou errno) est compare a ce
 * que le role DOIT obtenir.
 *
 *   matrice-roles-probe prepare      hors bac a sable : depose les cibles
 *   <role> (meme binaire)            confine : joue la matrice de son role
 *
 * Roles : WebContent, WebWorker, ImageDecoder, Compositor (BrowserContent :
 * rendu), RequestServer (BrowserNetwork : reseau + cache HTTP).
 *
 * Exception DOCUMENTEE : /dev/dsp en ECRITURE pour le profil de rendu
 * (LibMedia joue le son dans WebContent ; le profil est partage par les
 * quatre roles de rendu -- docs/ladybird/SECURITE_ROLES.md).
 *
 * Sortie : une ligne `MATRICE role= op= attendu= obtenu= ok|ECART` par
 * operation, puis `MATRICE_ROLE_OK role=... n=...` ou `MATRICE_ROLE_ECHEC`.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define DB "/persist/ladybird/data/Ladybird/Profiles/default/Ladybird.db"
#define REGLAGES "/persist/ladybird/config/Ladybird/Profiles/default/Settings.json"
#define CACHE_DIR "/persist/ladybird/cache/Ladybird/Profiles/default/Cache"
#define CACHE_ENTREE CACHE_DIR "/entree-matrice"
#define TMP_DB "/tmp/ladybird-data/Ladybird/Profiles/default/Ladybird.db"
#define TMP_CACHE "/tmp/ladybird-cache/Ladybird/Profiles/default/Cache/entree-matrice"
#define TELECHARGEMENT "/persist/Downloads/document.pdf"
#define HISTORIQUE "/persist/ladybird-chrome/historique"

enum { REFUS = 0, PERMIS = 1 };

static int ecarts = 0;
static int verifs = 0;
static const char *role = "?";

static void mkdir_p(const char *chemin)
{
    char partiel[256];
    for (size_t i = 1; chemin[i - 1]; i++) {
        if (chemin[i] == '/' || chemin[i] == '\0') {
            memcpy(partiel, chemin, i);
            partiel[i] = '\0';
            mkdir(partiel, 0755);
        }
    }
}

static void depose(const char *chemin, const char *contenu)
{
    char dossier[256];
    snprintf(dossier, sizeof dossier, "%s", chemin);
    char *fin = strrchr(dossier, '/');
    if (fin) {
        *fin = '\0';
        mkdir_p(dossier);
    }
    int fd = open(chemin, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd >= 0) {
        write(fd, contenu, strlen(contenu));
        close(fd);
    }
    printf("matrice-roles-probe prepare : %s %s\n", chemin, fd >= 0 ? "ok" : "ECHEC");
}

/* Compare `r` (>=0 : succes ; <0 : -errno) a l'attendu. */
static void juge(const char *op, int attendu, int r)
{
    verifs++;
    int obtenu = r >= 0 ? PERMIS : REFUS;
    int ok = obtenu == attendu;
    // Un refus doit etre un refus de SECURITE, pas une absence de fichier
    // (ENOENT prouverait seulement que la cible manque).
    if (ok && obtenu == REFUS && -r != EACCES && -r != EPERM)
        ok = 0;
    printf("MATRICE role=%s op=%s attendu=%s obtenu=%s",
        role, op, attendu == PERMIS ? "PERMIS" : "REFUS", obtenu == PERMIS ? "PERMIS" : "REFUS");
    if (r < 0)
        printf("(errno=%d)", -r);
    printf(" %s\n", ok ? "ok" : "ECART");
    ecarts += !ok;
}

static int ouvre(const char *chemin, int drapeaux)
{
    int fd = open(chemin, drapeaux, 0600);
    if (fd < 0)
        return -errno;
    close(fd);
    return 0;
}

static int prise_reseau(void)
{
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0)
        return -errno;
    close(s);
    return 0;
}

static int lance_un_programme(void)
{
    pid_t p = fork();
    if (p < 0)
        return -errno;
    if (p == 0) {
        execl("/bin/matrice-roles-probe", "matrice-roles-probe", "cible", (char *)0);
        _exit(100 + (errno & 0x3f));
    }
    int st = 0;
    waitpid(p, &st, 0);
    if (WIFEXITED(st) && WEXITSTATUS(st) >= 100)
        return -(WEXITSTATUS(st) - 100);
    return 0;
}

int main(int argc, char **argv)
{
    // Cible de l'essai d'exec : un programme qui EXISTE, et ne fait rien.
    if (argc > 1 && strcmp(argv[1], "cible") == 0)
        return 0;
    if (argc > 1 && strcmp(argv[1], "prepare") == 0) {
        depose(DB, "SQLite format 3 (factice)");
        depose(REGLAGES, "{\"factice\":true}");
        depose(CACHE_ENTREE, "reponse en cache");
        depose(TMP_DB, "SQLite format 3 (factice, ephemere)");
        depose(TMP_CACHE, "reponse en cache (ephemere)");
        depose(TELECHARGEMENT, "%PDF-1.4 factice");
        depose(HISTORIQUE, "https://exemple.invalid/");
        return 0;
    }

    const char *base = strrchr(argv[0], '/');
    role = base ? base + 1 : argv[0];
    int reseau = strcmp(role, "RequestServer") == 0;
    int rendu = !reseau;

    int confine = prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0);
    printf("matrice-roles-probe : role=%s no_new_privs=%d\n", role, confine);
    verifs++;
    if (confine != 1) {
        printf("MATRICE role=%s op=confine attendu=1 obtenu=%d ECART\n", role, confine);
        ecarts++;
    }

    // Le profil prive du navigateur : base SQL (cookies, stockage local) et
    // reglages appartiennent au processus navigateur, A PERSONNE d'autre.
    juge("lire-base-sql", REFUS, ouvre(DB, O_RDONLY));
    juge("ecrire-base-sql", REFUS, ouvre(DB, O_WRONLY));
    juge("lire-reglages", REFUS, ouvre(REGLAGES, O_RDONLY));
    juge("lire-base-sql-ephemere", REFUS, ouvre(TMP_DB, O_RDONLY));
    // Le cache HTTP : a RequestServer seul.
    juge("lire-cache-http", reseau ? PERMIS : REFUS, ouvre(CACHE_ENTREE, O_RDONLY));
    juge("ecrire-cache-http", reseau ? PERMIS : REFUS, ouvre(CACHE_DIR "/nouvelle-entree", O_WRONLY | O_CREAT));
    juge("lire-cache-http-ephemere", reseau ? PERMIS : REFUS, ouvre(TMP_CACHE, O_RDONLY));
    // Telechargements et historique/favoris : au processus navigateur.
    juge("lire-telechargement", REFUS, ouvre(TELECHARGEMENT, O_RDONLY));
    juge("lire-historique", REFUS, ouvre(HISTORIQUE, O_RDONLY));
    juge("ecrire-systeme", REFUS, ouvre("/usr/share/ladybird/implant", O_WRONLY | O_CREAT));
    juge("ecrire-racine-persist", REFUS, ouvre("/persist/implant", O_WRONLY | O_CREAT));
    // Le son : ecriture seule, pour le rendu (exception documentee).
    juge("ecrire-dev-dsp", rendu ? PERMIS : REFUS, ouvre("/dev/dsp", O_WRONLY));
    juge("lire-dev-dsp", REFUS, ouvre("/dev/dsp", O_RDONLY));
    // Le reseau : RequestServer seul.
    juge("socket-inet", reseau ? PERMIS : REFUS, prise_reseau());
    // Aucun role sandboxe ne lance de programme.
    juge("exec-programme", REFUS, lance_un_programme());

    if (ecarts == 0)
        printf("MATRICE_ROLE_OK role=%s n=%d\n", role, verifs);
    else
        printf("MATRICE_ROLE_ECHEC role=%s ecarts=%d n=%d\n", role, ecarts, verifs);
    return ecarts != 0;
}
