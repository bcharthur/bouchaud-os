/*
 * ata-concurrence-probe : des lectures disque concurrentes, en masse, pour
 * que le verrou du controleur ATA change de main des milliers de fois.
 *
 * POURQUOI CETTE SONDE
 *
 * Un blocage a ete observe (endurance SMP4, lecture DMA) : cinq lecteurs
 * parques sur la file du controleur, un sixieme `Blocked` SANS cle d'attente,
 * hors de toute file d'execution -- un reveil perdu. Il sortait une fois sur
 * dix-huit cycles : trop rare pour qu'un avant/apres prouve quoi que ce soit.
 *
 * Chaque lecture de 4 Kio a une position pseudo-aleatoire d'un fichier de
 * 32 Mio sort du cache de lecture (4 Mio) et prend le verrou du controleur ;
 * huit lecteurs concurrents le font tourner en file, avec un reveil a chaque
 * liberation. Un reveil perdu fige la file : le parent n'obtient plus jamais
 * ses enfants, et le cycle expire au lieu de publier son marqueur.
 *
 *     ata-concurrence-probe FICHIER [LECTEURS] [TOURS]
 *
 * Verdict : `ATA_CONCURRENCE_OK lecteurs=N lectures=M` si tous les lecteurs
 * sont revenus avec toutes leurs lectures completes ; sinon
 * `ATA_CONCURRENCE_FAIL` et code 1.
 */
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define LECTEURS_MAX 32

static int lecteur(const char *chemin, int graine, int tours)
{
    int fd = open(chemin, O_RDONLY);
    if (fd < 0)
        return 1;
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size < 1048576) {
        close(fd);
        return 1;
    }
    unsigned long pages = (unsigned long)st.st_size / 4096;
    unsigned long x = 2463534242UL + (unsigned long)graine * 7919UL;
    char tampon[4096];
    for (int i = 0; i < tours; i++) {
        /* xorshift : dispersion sans dependance a la libc. */
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        off_t ou = (off_t)((x % pages) * 4096);
        if (pread(fd, tampon, sizeof(tampon), ou) != (ssize_t)sizeof(tampon)) {
            close(fd);
            return 1;
        }
    }
    close(fd);
    return 0;
}

int main(int argc, char **argv)
{
    if (argc < 2) {
        printf("ATA_CONCURRENCE_FAIL usage: ata-concurrence-probe FICHIER [LECTEURS] [TOURS]\n");
        return 1;
    }
    int n = argc > 2 ? atoi(argv[2]) : 8;
    int tours = argc > 3 ? atoi(argv[3]) : 400;
    if (n < 1 || n > LECTEURS_MAX || tours < 1) {
        printf("ATA_CONCURRENCE_FAIL parametres\n");
        return 1;
    }
    pid_t fils[LECTEURS_MAX];
    int nes = 0;
    for (int i = 0; i < n; i++) {
        pid_t p = fork();
        if (p == 0)
            _exit(lecteur(argv[1], i, tours));
        if (p > 0)
            fils[nes++] = p;
    }
    int echecs = nes == n ? 0 : n - nes;
    for (int i = 0; i < nes; i++) {
        int statut = 0;
        if (waitpid(fils[i], &statut, 0) != fils[i] || !WIFEXITED(statut) || WEXITSTATUS(statut) != 0)
            echecs++;
    }
    if (echecs != 0) {
        printf("ATA_CONCURRENCE_FAIL lecteurs=%d echecs=%d\n", n, echecs);
        return 1;
    }
    printf("ATA_CONCURRENCE_OK lecteurs=%d lectures=%d\n", n, n * tours);
    return 0;
}
