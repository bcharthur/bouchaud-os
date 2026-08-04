/*
 * Cible d'`execve` utilisee par `posix-probe`.
 *
 * Verifie que le remplacement d'image transmet bien argv et envp, puis rend un
 * code de sortie reconnaissable par l'appelant.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv)
{
    const char *marqueur = getenv("MARQUEUR");
    printf("    [exec-cible] argc=%d argv[1]=%s MARQUEUR=%s\n",
           argc, argc > 1 ? argv[1] : "(aucun)", marqueur ? marqueur : "(absent)");

    int correct = argc == 2
        && strcmp(argv[1], "bonjour") == 0
        && marqueur != NULL
        && strcmp(marqueur, "transmis") == 0;

    /* 7 = tout est arrive intact ; toute autre valeur signale un probleme. */
    return correct ? 7 : 1;
}
