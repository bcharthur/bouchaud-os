/*
 * Sonde : l'extinction tombe pendant que des processus orphelins tiennent la
 * persistance et le disque.
 *
 * BOUCHAUD_EXTINCTION_FIL_NOYAU_V1
 *
 * Sites reels, run 38475302676 (d466c9ff) : apres `=== AUTORUN FIN ===`,
 * l'extinction part du contexte d'amorcage -- aucune tache courante -- et
 * `persistance::synchronise` y attend un verrou dormant (`TRANSACTION`, ou le
 * controleur ATA) tenu par un processus Ladybird encore vivant. Une attente
 * dormante exige une tache :
 *
 *     panicked at src/kernel/process/thread/blocage.rs:105:5:
 *     task: parking demande hors de toute tache
 *
 * Ici, N fils orphelins bouclent sur ecriture + fsync sous /persist (chaque
 * fsync prend `TRANSACTION` et le controleur), puis le pere sort tout de
 * suite : l'autorun se termine et l'extinction tombe en pleine contention.
 *
 * Usage : extinction-contention-probe [fils]   (defaut 3)
 * Sortie : EXTINCTION_CONTENTION_PRETE fils=N ; le verdict est l'extinction
 * elle-meme (aucune panique, `BOUCHAUD_EXTINCTION_RAPPORT`).
 */
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static void ecrivain(int n)
{
    char chemin[64];
    char bloc[4096];
    memset(bloc, 'a' + n, sizeof bloc);
    snprintf(chemin, sizeof chemin, "/persist/extinction-contention-%d", n);
    for (unsigned long i = 0;; i++) {
        int fd = open(chemin, O_CREAT | O_WRONLY | O_TRUNC, 0644);
        if (fd < 0)
            _exit(2);
        for (int k = 0; k < 8; k++)
            (void)!write(fd, bloc, sizeof bloc);
        fsync(fd);
        close(fd);
    }
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    int fils = argc > 1 ? atoi(argv[1]) : 3;
    if (fils < 1)
        fils = 1;
    mkdir("/persist", 0755);
    for (int n = 0; n < fils; n++) {
        pid_t p = fork();
        if (p == 0)
            ecrivain(n);
        if (p < 0) {
            printf("EXTINCTION_CONTENTION_ECHEC fork n=%d\n", n);
            return 1;
        }
    }
    /* Laisser les fils entrer dans leur boucle d'ecriture. */
    usleep(300 * 1000);
    printf("EXTINCTION_CONTENTION_PRETE fils=%d\n", fils);
    return 0;
}
