// Sonde : longueur maximale d'un nom de fichier, et erreur rendue au-dela.
//
// Ladybird ecrit chaque telechargement dans un fichier temporaire nomme
// `<fichier>.<numero>.<uuid>.download`. L'UUID fait 36 octets, le suffixe
// complet 48. Avec `preuve-bouchaud.bin`, cela donne 67 octets -- au-dela de
// l'ancien plafond de 64 de Bouchaud, et la creation echouait en annoncant
// ENOSPC. Aucun telechargement n'aboutissait, et le journal accusait un disque
// plein de 1166 Mio dont 4 % des inodes etaient pris (run 32427953935).
//
// La sonde verifie donc DEUX choses, que Linux garantit toutes les deux :
//   - un nom de 255 octets est accepte ;
//   - un nom de 256 octets est refuse par ENAMETOOLONG, pas par ENOSPC.

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-52s %s\n", quoi, condition ? "ok" : "ECHEC");
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

/* Cree `base/<n octets>` et rend errno (0 si la creation a reussi). */
static int cree_nom_de(const char *base, size_t n, int *cree)
{
    char chemin[512];
    size_t tete = (size_t)snprintf(chemin, sizeof(chemin), "%s/", base);
    for (size_t i = 0; i < n && tete + i + 1 < sizeof(chemin); i++)
        chemin[tete + i] = 'n';
    chemin[tete + n] = '\0';

    errno = 0;
    int fd = open(chemin, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0644);
    if (fd < 0) {
        *cree = 0;
        return errno;
    }
    close(fd);
    unlink(chemin);
    *cree = 1;
    return 0;
}

int main(void)
{
    printf("Sonde longueur des noms de fichier (NAME_MAX)\n\n");
    const char *base = racine();
    printf("  repertoire d'essai : %s\n\n", base);

    int cree = 0;
    int err = cree_nom_de(base, 255, &cree);
    printf("      255 octets : %s\n", cree ? "cree" : strerror(err));
    verifie("un nom de 255 octets est accepte", cree == 1);

    err = cree_nom_de(base, 256, &cree);
    printf("      256 octets : %s\n", cree ? "cree (!)" : strerror(err));
    verifie("un nom de 256 octets est refuse", cree == 0);
    verifie("le refus est ENAMETOOLONG et non ENOSPC", cree == 0 && err == ENAMETOOLONG);

    /* Le cas exact de Ladybird : basename + suffixe temporaire. */
    char temporaire[512];
    snprintf(temporaire, sizeof(temporaire), "%s/%s", base,
        "preuve-bouchaud.bin.1.550e8400-e29b-41d4-a716-446655440000.download");
    printf("\n      nom temporaire Ladybird (%zu octets) : %s\n",
        strlen("preuve-bouchaud.bin.1.550e8400-e29b-41d4-a716-446655440000.download"),
        temporaire);
    errno = 0;
    int fd = open(temporaire, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0644);
    verifie("le nom temporaire d'un telechargement est creable", fd >= 0);
    if (fd < 0) {
        printf("      refuse : %s\n", strerror(errno));
    } else {
        const char contenu[] = "Bouchaud download proof";
        verifie("il s'ecrit", write(fd, contenu, sizeof(contenu) - 1) == (ssize_t)(sizeof(contenu) - 1));
        close(fd);
        /* Et il se renomme vers sa destination finale, comme le fait
         * FileDownloader quand le transfert s'acheve. */
        char final[512];
        snprintf(final, sizeof(final), "%s/preuve-nom-long.bin", base);
        verifie("il se renomme vers sa destination", rename(temporaire, final) == 0);
        unlink(final);
        unlink(temporaire);
    }

    /* Une table d'inodes epuisee doit, elle, rendre ENOSPC : la sonde ne peut
     * pas l'atteindre sans remplir le systeme de fichiers, mais elle verifie au
     * moins que les creations repetees et refusees n'en consomment pas. */
    for (int i = 0; i < 200; i++) {
        int jete = 0;
        (void)cree_nom_de(base, 256, &jete);
    }
    err = cree_nom_de(base, 255, &cree);
    printf("\n      apres 200 refus, un nom valide : %s\n", cree ? "toujours creable" : strerror(err));
    verifie("200 refus ne consomment aucun inode", cree == 1);

    printf("\nRESULTAT : %d verification(s) en echec\n", echecs);
    printf("NOM_LONG_%s echecs=%d\n", echecs == 0 ? "OK" : "FAIL", echecs);
    return echecs == 0 ? 0 : 1;
}
