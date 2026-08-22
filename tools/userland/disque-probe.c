/*
 * Sonde d'acces concurrent au disque de donnees.
 *
 * Le pilote ATA de Bouchaud parle en PIO : pour chaque commande il selectionne
 * le disque, ecrit les registres LBA, envoie l'ordre, puis transfere les
 * donnees mot par mot. C'est un protocole a plusieurs etapes sur un peripherique
 * unique et partage.
 *
 * Rien ne le protege. L'ordonnanceur, lui, preempte : un minuteur peut
 * interrompre une tache au milieu de son transfert, en lancer une autre qui
 * commence son propre transfert, et les deux se retrouvent a piloter le meme
 * controleur en meme temps. La commande de l'une s'execute avec les registres
 * de l'autre.
 *
 * C'est ce qui a tue le BrowserHost au run 32426569316 :
 *
 *   [kernel] persistance: ecriture de 'ladybird/profile/data/Ladybird.db-journal'
 *            incomplete, 0 secteurs sur 1 a partir de 2128427
 *   [syscall-echec] 74 (fsync) = -5 (EIO)
 *
 * Cinq processus -- BrowserHost, WebContent, RequestServer, ImageDecoder,
 * Compositor -- paginaient a la demande leurs 190 Mio depuis le disque pendant
 * que SQLite validait ses transactions. Le premier `fsync` qui tombe au mauvais
 * moment rend EIO, SQLite le traduit en « disk I/O error », et l'hote meurt.
 *
 * La sonde reproduit ce melange : des lecteurs qui paginent un gros fichier
 * pendant qu'un ecrivain valide sous /persist. Sans exclusion mutuelle dans le
 * pilote, une operation au moins rend un compte court.
 *
 *   musl-gcc -O2 -static-pie disque-probe.c -o disque-probe
 *   (ou via ./build.sh musl)
 */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define LECTEURS 5
#define TOURS_ECRITURE 120
#define TOURS_LECTURE 1500

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-52s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

/* Lit des tranches dispersees d'un gros fichier : chaque lecture qui sort du
 * cache oblige le noyau a redescendre jusqu'au controleur. */
static int lit_disperse(const char *chemin, int tours)
{
    int fd = open(chemin, O_RDONLY);
    if (fd < 0)
        return 0;

    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size < 65536) {
        close(fd);
        return 0;
    }

    char tampon[4096];
    for (int i = 0; i < tours; i++) {
        off_t ou = (off_t)((i * 977) % (st.st_size / 4096)) * 4096;
        if (lseek(fd, ou, SEEK_SET) != ou) { close(fd); return 0; }
        ssize_t n = read(fd, tampon, sizeof(tampon));
        if (n != (ssize_t)sizeof(tampon)) { close(fd); return 0; }
    }
    close(fd);
    return 1;
}

int main(int argc, char **argv)
{
    printf("Sonde d'acces concurrent au disque\n\n");

    const char *gros = argc > 1 ? argv[1] : "/usr/libexec/ladybird/WebContent";
    struct stat st;
    int a_gros = stat(gros, &st) == 0 && st.st_size > 1048576;
    printf("  fichier a paginer : %s%s\n", gros, a_gros ? "" : " (absent : lecteurs inactifs)");

    struct stat sp;
    int a_persist = stat("/persist", &sp) == 0 && S_ISDIR(sp.st_mode);
    printf("  zone persistante  : %s\n\n", a_persist ? "/persist" : "ABSENTE");
    if (!a_persist) {
        printf("DISQUE_PROBE_FAIL echecs=1 (pas de zone persistante)\n");
        return 1;
    }

    /* Les lecteurs tournent pendant que l'ecrivain valide. */
    pid_t lecteurs[LECTEURS];
    int nes = 0;
    if (a_gros) {
        for (int i = 0; i < LECTEURS; i++) {
            pid_t p = fork();
            if (p == 0) {
                _exit(lit_disperse(gros, TOURS_LECTURE) ? 0 : 1);
            }
            if (p > 0)
                lecteurs[nes++] = p;
        }
    }
    printf("[ecritures sous /persist pendant %d lecteur(s)]\n", nes);

    int ecritures_ok = 0, fsync_ok = 0;
    int premier_echec = -1;
    for (int i = 0; i < TOURS_ECRITURE; i++) {
        char chemin[128];
        snprintf(chemin, sizeof(chemin), "/persist/disque-probe-%d.txt", i % 4);
        int fd = open(chemin, O_RDWR | O_CREAT | O_TRUNC, 0644);
        if (fd < 0)
            continue;
        /* 256 Kio par fichier : `synchronise` reecrit TOUTE la zone a chaque
         * `fsync`, donc plus elle porte de donnees, plus le pilote reste
         * longtemps dans son transfert -- et plus il expose sa fenetre. */
        static char contenu[256 * 1024];
        memset(contenu, 'a' + (i % 26), sizeof(contenu));
        if (write(fd, contenu, sizeof(contenu)) == (ssize_t)sizeof(contenu))
            ecritures_ok++;
        if (fsync(fd) == 0) {
            fsync_ok++;
        } else if (premier_echec < 0) {
            premier_echec = i;
            printf("      premier fsync en echec au tour %d : %s\n", i, strerror(errno));
        }
        close(fd);
    }

    int lecteurs_ok = 0;
    for (int i = 0; i < nes; i++) {
        int statut = 0;
        waitpid(lecteurs[i], &statut, 0);
        if (WIFEXITED(statut) && WEXITSTATUS(statut) == 0)
            lecteurs_ok++;
    }

    printf("      ecritures %d/%d, fsync %d/%d, lecteurs %d/%d\n",
        ecritures_ok, TOURS_ECRITURE, fsync_ok, TOURS_ECRITURE, lecteurs_ok, nes);

    verifie("toutes les ecritures ont abouti", ecritures_ok == TOURS_ECRITURE);
    verifie("tous les fsync ont abouti", fsync_ok == TOURS_ECRITURE);
    verifie("tous les lecteurs ont abouti", lecteurs_ok == nes);

    for (int i = 0; i < 4; i++) {
        char chemin[128];
        snprintf(chemin, sizeof(chemin), "/persist/disque-probe-%d.txt", i);
        unlink(chemin);
    }

    printf("\nRESULTAT : %d verification(s) en echec\n", echecs);
    printf("DISQUE_PROBE_%s echecs=%d\n", echecs == 0 ? "OK" : "FAIL", echecs);
    return echecs == 0 ? 0 : 1;
}
