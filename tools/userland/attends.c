/*
 * attends SECONDES -- dort, puis rend 0.
 *
 * BOUCHAUD_BUREAU_RING3_2A : l'autorun d'un banc s'execute dans le contexte
 * d'amorcage, qui ne peut pas dormir lui-meme, et la console des processus
 * n'atteint la serie que tant que l'autorun tourne. Un banc dont les acteurs
 * vivent hors de l'autorun (compositeur, application) le garde ouvert ainsi.
 *
 *   musl-gcc -O2 -static-pie -fPIE -o attends attends.c
 */
#include <stdlib.h>
#include <time.h>

int main(int argc, char **argv)
{
    long s = argc > 1 ? strtol(argv[1], 0, 10) : 1;
    if (s < 0) s = 0;
    struct timespec t = { s, 0 };
    while (nanosleep(&t, &t) != 0) { }
    return 0;
}
