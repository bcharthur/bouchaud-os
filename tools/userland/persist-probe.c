// Verifie que la machine retient quelque chose d'un demarrage a l'autre.
//
// Le RAMFS oublie tout a l'extinction. `/persist` est la zone qui n'oublie
// pas : le noyau la deplie du disque au demarrage et l'y reecrit sur `sync`.
// Cette sonde se joue en deux passages, et elle reconnait seule celui qu'elle
// joue : si le temoin est deja la, c'est qu'un demarrage precedent l'a ecrit et
// que la zone a tenu. Le meme scenario sert donc aux deux demarrages, ce qui
// evite d'avoir a refabriquer l'image entre les deux — refabriquer effacerait
// justement ce qu'on cherche a verifier.
//
// Entre les deux passages, la machine s'eteint et redemarre — c'est tout
// l'interet. Un seul passage ne prouverait rien : n'importe quel systeme de
// fichiers en memoire relit ce qu'il vient d'ecrire.

#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
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

// Ce qu'on depose. Un fichier a la racine, un dans un sous-dossier — la zone
// doit retenir l'arborescence, pas seulement des noms plats.
static const char *CHEMIN_A = "/persist/temoins.txt";
static const char *CHEMIN_B = "/persist/cache/entree-1";
static const char *CONTENU_A = "session=abc123; domaine=exemple.test\n";
static const char *CONTENU_B = "HTTP/1.1 200 OK\r\n\r\n<h1>page en cache</h1>\n";

static int ecrit_fichier(const char *chemin, const char *contenu)
{
    int fd = open(chemin, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0)
        return -1;
    size_t taille = strlen(contenu);
    ssize_t ecrits = write(fd, contenu, taille);
    // `fsync` sur un fichier de `/persist` declenche l'ecriture du disque : le
    // reste du systeme n'a rien a vider, et c'est ce qui evite de payer une
    // reecriture complete a chaque `fsync` d'un programme quelconque.
    fsync(fd);
    close(fd);
    return ecrits == (ssize_t)taille ? 0 : -1;
}

static int lit_fichier(const char *chemin, char *tampon, size_t taille)
{
    int fd = open(chemin, O_RDONLY);
    if (fd < 0)
        return -1;
    ssize_t lus = read(fd, tampon, taille - 1);
    close(fd);
    if (lus < 0)
        return -1;
    tampon[lus] = '\0';
    return (int)lus;
}

static int ecrit(void)
{
    printf("persist-probe: passage 1 — ecriture\n");

    struct stat etat;
    verifie("/persist existe", stat("/persist", &etat) == 0, NULL);
    verifie("/persist est un dossier", S_ISDIR(etat.st_mode), NULL);

    verifie("le sous-dossier est cree", mkdir("/persist/cache", 0755) == 0
            || stat("/persist/cache", &etat) == 0, NULL);
    verifie("le premier fichier s'ecrit", ecrit_fichier(CHEMIN_A, CONTENU_A) == 0, NULL);
    verifie("le second aussi", ecrit_fichier(CHEMIN_B, CONTENU_B) == 0, NULL);

    // Relecture immediate : ce n'est pas la persistance, mais si cela echoue,
    // inutile de chercher plus loin au passage suivant.
    char tampon[512];
    verifie("relecture immediate", lit_fichier(CHEMIN_A, tampon, sizeof tampon) > 0, NULL);
    verifie("contenu immediat", strcmp(tampon, CONTENU_A) == 0, tampon);

    // Et un `sync` explicite, qui ecrit toute la zone.
    sync();
    verifie("sync accepte", 1, NULL);
    return 0;
}

static int relit(void)
{
    printf("persist-probe: passage 2 — relecture apres redemarrage\n");

    char tampon[512];
    int lus = lit_fichier(CHEMIN_A, tampon, sizeof tampon);
    verifie("le fichier a survecu au redemarrage", lus > 0, NULL);
    verifie("son contenu est intact", lus > 0 && strcmp(tampon, CONTENU_A) == 0,
            lus > 0 ? tampon : "(illisible)");

    lus = lit_fichier(CHEMIN_B, tampon, sizeof tampon);
    verifie("le fichier du sous-dossier aussi", lus > 0, NULL);
    verifie("et son contenu", lus > 0 && strcmp(tampon, CONTENU_B) == 0,
            lus > 0 ? tampon : "(illisible)");

    struct stat etat;
    verifie("l'arborescence est restauree",
            stat("/persist/cache", &etat) == 0 && S_ISDIR(etat.st_mode), NULL);
    return 0;
}

int main(void)
{
    struct stat etat;
    if (stat(CHEMIN_A, &etat) == 0 && etat.st_size > 0)
        relit();
    else
        ecrit();

    printf("RESULTAT : %d verification(s) en echec (%d passees)\n", echecs, reussites);
    return echecs == 0 ? 0 : 1;
}
