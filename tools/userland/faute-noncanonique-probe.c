/*
 * Sonde : une faute utilisateur sur une adresse NON CANONIQUE tue le
 * programme fautif, pas le noyau.
 *
 * BOUCHAUD_CR2_BRUT_V1
 *
 * Smoke 37518121906 : WebContent a touche 0xffff413f021ee480 et le noyau a
 * panique (`VirtAddr::new ... VirtAddrNotValid(0xffff413f021ee480)`) : le
 * gestionnaire de faute lisait CR2 par `Cr2::read()` (x86_64 0.14), qui
 * panique sur une adresse non canonique. Sous QEMU TCG l'acces arrive en
 * faute de PAGE avec ce CR2 ; sur materiel il arrive en #GP, dont le chemin
 * lisait aussi CR2.
 *
 * Trois fils : lecture, ecriture, saut vers l'adresse. Chacun doit mourir de
 * SIGSEGV ; le pere doit survivre et le dire.
 *
 * Sortie : `FAUTE_NONCANONIQUE_OK` ou `FAUTE_NONCANONIQUE_ECHEC n=...`.
 */
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/wait.h>
#include <unistd.h>

#define ADRESSE 0xffff413f021ee480ull

int main(void)
{
    static const char *const noms[] = { "lecture", "ecriture", "saut" };
    int echecs = 0;
    for (int essai = 0; essai < 3; essai++) {
        pid_t p = fork();
        if (p < 0) {
            echecs++;
            continue;
        }
        if (p == 0) {
            volatile uint64_t *x = (volatile uint64_t *)(uintptr_t)ADRESSE;
            if (essai == 0)
                printf("lu %llx\n", (unsigned long long)*x);
            else if (essai == 1)
                *x = 1;
            else
                ((void (*)(void))(uintptr_t)ADRESSE)();
            _exit(0);
        }
        int st = 0;
        waitpid(p, &st, 0);
        int ok = WIFSIGNALED(st) && WTERMSIG(st) == SIGSEGV;
        printf("  %-10s status=%#x %s\n", noms[essai], st, ok ? "ok (SIGSEGV)" : "ECHEC");
        echecs += !ok;
    }
    if (echecs == 0)
        printf("FAUTE_NONCANONIQUE_OK\n");
    else
        printf("FAUTE_NONCANONIQUE_ECHEC n=%d\n", echecs);
    return echecs != 0;
}
