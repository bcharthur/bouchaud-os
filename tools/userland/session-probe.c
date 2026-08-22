// Sonde : un programme de premier plan qui s'arrete DOIT rendre la main a
// l'invite, meme s'il laisse des fils vivants.
//
// C'est la forme exacte de Ladybird. `BouchaudBrowserHost` quitte proprement
// sur `window.close()`, mais ses services -- WebContent, RequestServer,
// ImageDecoder, Compositor -- restent dans leur boucle d'evenements : upstream
// les fait mourir avec leur parent par PR_SET_PDEATHSIG, que Bouchaud ne
// fournit pas encore. Au run 32427953935 l'invite n'est jamais revenue :
// `task::run` n'avait qu'un critere de retour, « plus aucune tache
// executable », et des fils qui tournent l'empechent a jamais. L'autorun ne se
// terminait donc pas, `power::shutdown` n'etait jamais appele, et /persist
// n'etait jamais ecrit a l'extinction.
//
// La sonde ne peut pas constater elle-meme qu'elle a rendu la main -- elle
// n'est plus la pour le dire. C'est l'AUTORUN qui le constate : la commande
// suivante s'execute, ou bien la machine reste bloquee jusqu'a son echeance.
// La sonde ecrit donc son marqueur, laisse ses fils derriere elle, et sort.

#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#include <time.h>
#include <string.h>
#include <unistd.h>

#define FILS 4

int main(int argc, char **argv)
{
    int combien = argc > 1 ? atoi(argv[1]) : FILS;
    if (combien < 1)
        combien = 1;

    printf("Sonde fin de session : un pere qui sort en laissant %d fils\n\n", combien);

    for (int i = 0; i < combien; i++) {
        pid_t p = fork();
        if (p < 0) {
            printf("  fork() impossible : %s\n", strerror(errno));
            printf("SESSION_FAIL echecs=1\n");
            return 1;
        }
        if (p == 0) {
            // Un service : il ne se termine jamais de lui-meme, et il reste
            // EXECUTABLE -- c'est ce qui distingue ce cas d'un fils endormi.
            // Une boucle d'evenements qui interroge ses descripteurs se
            // comporte exactement ainsi.
            for (;;) {
                struct timespec court = { 0, 1000000 };
                nanosleep(&court, NULL);
            }
        }
        printf("  fils %d lance, pid %d (ne se terminera jamais)\n", i + 1, (int)p);
    }

    printf("\n  le pere sort maintenant, sans attendre ses fils\n");
    printf("SESSION_PERE_SORT fils=%d\n", combien);
    fflush(stdout);
    return 0;
}
