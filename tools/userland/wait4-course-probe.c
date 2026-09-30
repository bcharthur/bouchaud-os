/*
 * wait4-course-probe : des milliers de fils qui meurent aussitot nes, pour
 * que leur sortie tombe dans la fenetre de `wait4` entre « aucun zombie » et
 * « je me declare en attente ».
 *
 * POURQUOI CETTE SONDE
 *
 * `sys_wait4` cherche les zombies, n'en trouve pas, PUIS pose
 * `waiting_for_child` et `Blocked`. Un fils qui meurt entre les deux trouve
 * le drapeau a faux : `notify_parent_of_exit` ne reveille personne, et le
 * parent s'endort pour toujours (MESURE_DEMARRAGE_A_FROID §15). La course
 * etait decrite, jamais mesuree : aucune sonde ne la visait.
 *
 * Le fils sort immediatement ; sur un autre coeur, sa sortie court contre
 * l'entree du parent dans `wait4`. Le parent fait varier un court delai
 * avant l'appel pour balayer la fenetre. Un reveil perdu fige le parent :
 * le marqueur n'est jamais publie et le cycle expire.
 *
 *     wait4-course-probe [TOURS]
 *
 * Verdict : `WAIT4_COURSE_OK tours=N` ; sinon `WAIT4_COURSE_FAIL` et code 1.
 */
#include <stdio.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    int tours = argc > 1 ? atoi(argv[1]) : 2000;
    if (tours < 1) {
        printf("WAIT4_COURSE_FAIL parametres\n");
        return 1;
    }
    for (int i = 0; i < tours; i++) {
        pid_t p = fork();
        if (p < 0) {
            printf("WAIT4_COURSE_FAIL fork tour=%d\n", i);
            return 1;
        }
        if (p == 0)
            _exit(i & 0x7F);
        /* Delai variable : de zero a quelques milliers d'iterations. */
        for (volatile int j = 0; j < (i % 64) * 64; j++)
            ;
        int statut = 0;
        if (waitpid(p, &statut, 0) != p || !WIFEXITED(statut) || WEXITSTATUS(statut) != (i & 0x7F)) {
            printf("WAIT4_COURSE_FAIL tour=%d statut=%#x\n", i, statut);
            return 1;
        }
        if ((i + 1) % 500 == 0)
            printf("WAIT4_COURSE_PROGRES tours=%d\n", i + 1);
    }
    printf("WAIT4_COURSE_OK tours=%d\n", tours);
    return 0;
}
