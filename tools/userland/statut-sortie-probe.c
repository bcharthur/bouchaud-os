/*
 * statut-sortie-probe : `wait4` distingue une SORTIE de code 128..255 d'une
 * MORT PAR SIGNAL.
 *
 * POURQUOI CETTE SONDE
 *
 * Le noyau notait une mort par signal comme le code de sortie `128 + signal`
 * (la convention des shells), puis `wait4` decodait tout code >= 128 comme une
 * mort par signal. Un `exit(255)` -- ou `exit(130)` -- arrivait donc au pere
 * comme « tue par le signal 127 » (ou 2) : WIFEXITED faux, WEXITSTATUS
 * perdu. tube-atomicite-probe l'a rencontre (exit(250) lu comme signal 122).
 *
 * LA MESURE
 *
 * Des fils sortent avec 0, 1, 127, 128, 130, 200, 255 et -1 ; d'autres
 * meurent de SIGKILL (envoye par le pere), de SIGSEGV (faute) et de SIGABRT
 * (`abort`). Chaque statut doit se decoder exactement comme sous Linux.
 *
 *     statut-sortie-probe
 *
 * Verdict : `STATUT_SORTIE_OK cas=N` ; sinon `STATUT_SORTIE_FAIL` et code 1.
 */
#define _GNU_SOURCE
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <unistd.h>

static int echecs = 0, cas = 0;
/* Lue a l'execution : le compilateur ne peut pas changer la faute en `ud2`. */
static volatile unsigned long adresse_nulle = 0;

static void juge(const char *quoi, int statut, int sortie_attendue, int signal_attendu)
{
    int ok;
    if (signal_attendu)
        ok = WIFSIGNALED(statut) && WTERMSIG(statut) == signal_attendu;
    else
        ok = WIFEXITED(statut) && WEXITSTATUS(statut) == sortie_attendue;
    cas++;
    printf("STATUT_SORTIE %s statut=%#x exited=%d code=%d signaled=%d sig=%d %s\n", quoi, statut,
           WIFEXITED(statut), WEXITSTATUS(statut), WIFSIGNALED(statut), WTERMSIG(statut),
           ok ? "ok" : "ECHEC");
    if (!ok)
        echecs++;
}

static void sortie(int code)
{
    pid_t p = fork();
    if (p == 0)
        _exit(code);
    int st = 0;
    waitpid(p, &st, 0);
    char quoi[32];
    snprintf(quoi, sizeof quoi, "exit(%d)", code);
    juge(quoi, st, code & 0xff, 0);
}

int main(void)
{
    static const int codes[] = { 0, 1, 127, 128, 130, 200, 255, -1 };
    for (unsigned i = 0; i < sizeof codes / sizeof codes[0]; i++)
        sortie(codes[i]);

    pid_t p = fork();
    if (p == 0) {
        for (;;)
            pause();
    }
    usleep(50000);
    kill(p, SIGKILL);
    int st = 0;
    waitpid(p, &st, 0);
    juge("SIGKILL", st, 0, SIGKILL);

    p = fork();
    if (p == 0) {
        *(volatile int *)adresse_nulle = 1;
        _exit(0);
    }
    st = 0;
    waitpid(p, &st, 0);
    juge("SIGSEGV", st, 0, SIGSEGV);

    p = fork();
    if (p == 0)
        abort();
    st = 0;
    waitpid(p, &st, 0);
    juge("SIGABRT", st, 0, SIGABRT);

    if (echecs) {
        printf("STATUT_SORTIE_FAIL echecs=%d\n", echecs);
        return 1;
    }
    printf("STATUT_SORTIE_OK cas=%d\n", cas);
    return 0;
}
