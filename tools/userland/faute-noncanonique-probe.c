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
 * BOUCHAUD_PILE_DE_FAUTE_V1 : un quatrieme fils, `piege`, execute
 * `__builtin_trap()` (ud2) au bout de deux appels non inlines -- la forme
 * d'un `VERIFY` d'AK (ak_verification_failed -> ak_trap). Il doit mourir d'un
 * signal, et le noyau doit publier `PROCESS_FAULT_PILE` avec les adresses de
 * retour de `piege_niveau2` et `piege_niveau1` (verifie par le banc).
 *
 * Sortie : `FAUTE_NONCANONIQUE_OK` ou `FAUTE_NONCANONIQUE_ECHEC n=...`.
 */
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/wait.h>
#include <unistd.h>

#define ADRESSE 0xffff413f021ee480ull

__attribute__((noinline)) static void piege_niveau2(int n)
{
    if (n >= 0)
        __builtin_trap();
}

__attribute__((noinline)) static void piege_niveau1(int n)
{
    piege_niveau2(n + 1);
    __asm__ volatile("" ::: "memory"); /* pas d'appel terminal */
}

int main(void)
{
    static const char *const noms[] = { "lecture", "ecriture", "saut", "piege" };
    int echecs = 0;
    for (int essai = 0; essai < 4; essai++) {
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
            else if (essai == 2)
                ((void (*)(void))(uintptr_t)ADRESSE)();
            else
                piege_niveau1(essai);
            _exit(0);
        }
        int st = 0;
        waitpid(p, &st, 0);
        /* Le piege (ud2) : n'importe quel signal ; les autres : SIGSEGV. */
        int ok = WIFSIGNALED(st) && (essai == 3 || WTERMSIG(st) == SIGSEGV);
        printf("  %-10s status=%#x %s\n", noms[essai], st, ok ? "ok (signal)" : "ECHEC");
        echecs += !ok;
    }
    if (echecs == 0)
        printf("FAUTE_NONCANONIQUE_OK\n");
    else
        printf("FAUTE_NONCANONIQUE_ECHEC n=%d\n", echecs);
    return echecs != 0;
}
