/*
 * TOUCHER-PAGES : le plus petit programme qui produise des fautes de page.
 *
 * Il sert de CHARGE D'EPREUVE au livre de comptes des fautes
 * (`src/kernel/process/fautes.rs`), et c'est `tools/ci/run_fautes_demande.sh`
 * qui le lance.
 *
 * Sans lui, l'instrumentation des fautes ne se prouvait pas : le bureau, le
 * shell et les sondes du noyau sont des taches NOYAU, et le releve
 * `[SMP-PF] c0=0/0/0/0/0` le montre -- aucune faute utilisateur n'a lieu dans
 * un scenario QEMU ordinaire. Un compteur qu'aucun banc ne fait monter est un
 * compteur dont personne ne sait s'il marche.
 *
 * Il est ecrit sans bibliotheque -- `-nostdlib`, deux appels systeme ecrits a
 * la main -- pour une raison precise : ce qu'on veut mesurer est le cout des
 * fautes, et une libc en apporterait des siennes, au demarrage, avant meme
 * que le programme commence.
 */
/* Touche un grand tableau BSS page par page : chaque page est une promesse
   `Zero` que le noyau doit peupler a la demande. C'est exactement la branche
   `PromesseBacking::Zero` de `peuple_a_la_demande`. */
#define PAGES 4096
#define PAGE  4096
static char tableau[PAGES * PAGE];

static long appel(long n, long a, long b, long c) {
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return r;
}

void _start(void) {
    unsigned long somme = 0;
    for (int i = 0; i < PAGES; i++) {
        tableau[i * PAGE] = (char)(i & 0xff);
        somme += (unsigned char)tableau[i * PAGE];
    }
    char message[] = "toucher: 4096 pages ecrites\n";
    appel(1, 1, (long)message, sizeof(message) - 1);
    appel(60, (long)(somme & 1), 0, 0);
    for (;;) {}
}
