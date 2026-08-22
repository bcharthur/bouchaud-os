// Sonde : sendfile(2), l'appel par lequel RequestServer sert une reponse HTTP
// depuis son cache disque.
//
// `Core::System::transfer_file_through_socket` l'emploie sur Linux
// (Libraries/LibCore/System.cpp:901). L'appel manquait a Bouchaud, et au run
// 32474068384 le SECOND demarrage -- celui ou le cache existe deja -- rendait
// donc « RequestServer encountered an error reading a cached HTTP response »
// pour une requete HTTPS qui avait parfaitement abouti au demarrage precedent.
//
// La sonde verifie ce que la page de manuel promet, et que le code appelant
// suppose :
//   - une copie complete vers une socket ;
//   - avec un decalage explicite, la position du descripteur source NE BOUGE
//     PAS et `*offset` avance du nombre d'octets lus ;
//   - sans decalage, la position avance ;
//   - une source qui n'est pas un fichier ordinaire rend EINVAL ;
//   - un decalage au-dela de la fin rend zero, pas une erreur.

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/sendfile.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <unistd.h>

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-54s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

static const char *racine(void)
{
    struct stat st;
    if (stat("/persist", &st) == 0 && S_ISDIR(st.st_mode))
        return "/persist";
    return "/tmp";
}

#define TAILLE 5000

int main(void)
{
    printf("Sonde sendfile(2)\n\n");

    char chemin[256];
    snprintf(chemin, sizeof(chemin), "%s/sendfile-probe.bin", racine());

    static char contenu[TAILLE];
    for (int i = 0; i < TAILLE; i++)
        contenu[i] = (char)('A' + (i % 26));

    int src = open(chemin, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (src < 0) {
        printf("  open(%s) : %s\n", chemin, strerror(errno));
        printf("SENDFILE_FAIL echecs=1\n");
        return 1;
    }
    if (write(src, contenu, TAILLE) != TAILLE) {
        printf("  ecriture du fichier source impossible\n");
        printf("SENDFILE_FAIL echecs=1\n");
        return 1;
    }

    /* --- copie complete vers une socket, avec decalage explicite --------- */
    int paire[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, paire) != 0) {
        printf("  socketpair() : %s\n", strerror(errno));
        printf("SENDFILE_FAIL echecs=1\n");
        return 1;
    }

    off_t position_avant = lseek(src, 137, SEEK_SET);
    off_t decalage = 0;
    ssize_t envoyes = sendfile(paire[1], src, &decalage, TAILLE);
    printf("      sendfile(offset=0, count=%d) -> %zd, offset devient %lld\n",
        TAILLE, envoyes, (long long)decalage);
    verifie("sendfile copie les octets demandes", envoyes == TAILLE);
    verifie("*offset a avance d'autant", (long long)decalage == (long long)envoyes);
    verifie("la position du descripteur source n'a PAS bouge",
        lseek(src, 0, SEEK_CUR) == position_avant);

    static char relu[TAILLE];
    size_t total = 0;
    while (total < (size_t)envoyes) {
        ssize_t n = read(paire[0], relu + total, (size_t)envoyes - total);
        if (n <= 0)
            break;
        total += (size_t)n;
    }
    verifie("tout est arrive a l'autre bout", total == (size_t)envoyes);
    verifie("les octets sont identiques a la source",
        total == TAILLE && memcmp(relu, contenu, TAILLE) == 0);

    /* --- sans decalage : la position avance ------------------------------ */
    lseek(src, 1000, SEEK_SET);
    ssize_t seconds = sendfile(paire[1], src, NULL, 512);
    printf("      sendfile(NULL, count=512) depuis 1000 -> %zd, position %lld\n",
        seconds, (long long)lseek(src, 0, SEEK_CUR));
    verifie("sans offset, sendfile copie aussi", seconds == 512);
    verifie("sans offset, la position a avance", lseek(src, 0, SEEK_CUR) == 1512);

    static char suite[512];
    total = 0;
    while (total < 512) {
        ssize_t n = read(paire[0], suite + total, 512 - total);
        if (n <= 0)
            break;
        total += (size_t)n;
    }
    verifie("les octets partent bien du decalage courant",
        total == 512 && memcmp(suite, contenu + 1000, 512) == 0);

    /* --- au-dela de la fin : zero, pas une erreur ------------------------ */
    off_t trop_loin = TAILLE + 4096;
    errno = 0;
    ssize_t rien = sendfile(paire[1], src, &trop_loin, 128);
    printf("      sendfile au-dela de la fin -> %zd (%s)\n",
        rien, rien < 0 ? strerror(errno) : "pas d'erreur");
    verifie("un decalage au-dela de la fin rend zero", rien == 0);

    /* --- source qui n'est pas un fichier : EINVAL ------------------------ */
    int tube[2];
    if (pipe(tube) == 0) {
        errno = 0;
        ssize_t refus = sendfile(paire[1], tube[0], NULL, 16);
        printf("      sendfile depuis un tube -> %zd (%s)\n",
            refus, refus < 0 ? strerror(errno) : "accepte (!)");
        verifie("une source qui n'est pas un fichier rend EINVAL",
            refus < 0 && errno == EINVAL);
        close(tube[0]);
        close(tube[1]);
    }

    /* --- descripteur invalide : EBADF ------------------------------------ */
    errno = 0;
    ssize_t mauvais = sendfile(paire[1], 4242, NULL, 16);
    verifie("un descripteur source inconnu rend EBADF",
        mauvais < 0 && errno == EBADF);

    close(paire[0]);
    close(paire[1]);
    close(src);
    unlink(chemin);

    printf("\nRESULTAT : %d verification(s) en echec\n", echecs);
    printf("SENDFILE_%s echecs=%d\n", echecs == 0 ? "OK" : "FAIL", echecs);
    return echecs == 0 ? 0 : 1;
}
