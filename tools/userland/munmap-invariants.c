/*
 * BOUCHAUD_DEMONTAGE_PAR_TABLES_V1 -- INVARIANTS du demontage (munmap / brk)
 * apres le saut des tables de pages absentes.
 *
 *   munmap-invariants [cycles] [sans_fork]
 *
 * Chaque cycle, sur une reservation de 3 Gio + 5 pages (bornes non alignees) :
 *   - pages ecrites de part et d'autre de chaque frontiere de 2 Mio et de
 *     1 Gio presente dans la plage, plus des pages eparses ;
 *   - munmap PARTIELS non alignes : autour d'une frontiere de 2 Mio, a cheval
 *     sur une frontiere de 1 Gio, et un grand trou de ~700 Mio peu peuple ;
 *   - toute page ecrite HORS des trous garde son contenu ;
 *   - toute page (ecrite ou non) DANS un trou faute (lue par un enfant) ;
 *   - un mmap neuf au meme endroit rend des pages a zero.
 * Puis le tas : brk +64 Mio, pages ecrites, reduction a une borne NON alignee,
 * les pages sous la borne gardent leur contenu, celles au-dessus fautent ;
 * re-croissance : les pages rendues sont a ZERO (jamais l'ancien contenu).
 * Enfin : la memoire libre (sysinfo) revient a sa valeur de depart a quelques
 * pages pres -- aucun cadre perdu sur `cycles` cycles.
 */
#define _GNU_SOURCE
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/sysinfo.h>
#include <sys/wait.h>
#include <unistd.h>

#define PAGE 4096ull
#define MIO (1ull << 20)
#define GIO (1ull << 30)
#define BLOC (2 * MIO)

static int fautes;
static int echecs;
/* `sans_fork` : aucune lecture de controle dans un enfant -- isole le compte
 * de memoire du demontage de celui des processus de controle. */
static int sans_fork;
static int meme_adresse = 1;

/* 1 si la lecture de `p` faute. Le noyau tue le processus fautif (pas de
 * remise de SIGSEGV a un gestionnaire) : la lecture se fait dans un ENFANT.
 * fork recopie l'espace en parcourant les PTE (AddressSpace::duplicate,
 * iter_user_pages), pas les VMA : un PTE perime laisse par le demontage
 * serait herite, et l'enfant lirait la page -- le defaut serait vu. */
static int faute_en_lecture(volatile uint64_t *p)
{
    if (sans_fork)
        return 1;
    pid_t f = fork();
    if (f == 0) {
        (void)*p;
        _exit(0);
    }
    if (f < 0) {
        printf("MUNMAP_INV_ECHEC fork\n");
        return 0;
    }
    int st = 0;
    if (waitpid(f, &st, 0) != f)
        return 0;
    int faute = !(WIFEXITED(st) && WEXITSTATUS(st) == 0);
    fautes += faute;
    return faute;
}

static uint64_t motif(uint64_t adresse, int cycle)
{
    return adresse * 0x9E3779B97F4A7C15ull ^ (uint64_t)cycle;
}

static void echec(const char *quoi, uint64_t a)
{
    if (echecs < 20)
        printf("MUNMAP_INV_ECHEC %s adresse=0x%llx\n", quoi, (unsigned long long)a);
    echecs++;
}

static uint64_t libre(void)
{
    struct sysinfo si;
    if (sysinfo(&si) != 0)
        return 0;
    return (uint64_t)si.freeram * si.mem_unit;
}

#define MAX_PAGES 4096
static uint64_t ecrites[MAX_PAGES];
static int n_ecrites;

static void ecris(uint64_t a, int cycle)
{
    a &= ~(PAGE - 1);
    for (int i = 0; i < n_ecrites; i++)
        if (ecrites[i] == a)
            return;
    if (n_ecrites == MAX_PAGES)
        return;
    *(volatile uint64_t *)a = motif(a, cycle);
    ecrites[n_ecrites++] = a;
}

struct trou { uint64_t debut, fin; };

static int dans(uint64_t a, struct trou *t, int n)
{
    for (int i = 0; i < n; i++)
        if (a >= t[i].debut && a < t[i].fin)
            return 1;
    return 0;
}

static void cycle_mmap(int cycle)
{
    uint64_t taille = 3 * GIO + 5 * PAGE;
    /* Meme adresse a chaque cycle (indication, sans MAP_FIXED) : les tables
     * de pages du cycle d'amorce sont reutilisees -- le noyau ne rend les
     * tables videes qu'a la destruction de l'espace d'adressage. Le compte
     * de memoire juge alors les PAGES DE DONNEES rendues par le demontage. */
    static char *indication;
    char *p = mmap(indication, taille, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (p == MAP_FAILED) {
        echec("mmap", 0);
        return;
    }
    if (indication && p != indication)
        meme_adresse = 0;
    indication = p;
    uint64_t lo = (uint64_t)p, hi = lo + taille;
    n_ecrites = 0;
    /* Frontieres de 2 Mio (une sur 16, pour rester sous MAX_PAGES) et de 1 Gio. */
    for (uint64_t b = (lo + BLOC - 1) & ~(BLOC - 1); b < hi; b += 16 * BLOC) {
        if (b - PAGE >= lo) ecris(b - PAGE, cycle);
        if (b + PAGE <= hi) ecris(b, cycle);
    }
    for (uint64_t g = (lo + GIO - 1) & ~(GIO - 1); g < hi; g += GIO) {
        if (g - PAGE >= lo) ecris(g - PAGE, cycle);
        if (g + PAGE <= hi) ecris(g, cycle);
    }
    for (uint64_t a = lo; a < hi; a += 37 * MIO + 3 * PAGE)
        ecris(a, cycle);
    ecris(hi - PAGE, cycle);

    /* Trous non alignes. */
    struct trou t[3];
    int n = 0;
    uint64_t b2 = ((lo + 64 * MIO) & ~(BLOC - 1));
    t[n++] = (struct trou){ b2 - 3 * PAGE, b2 + 5 * PAGE };
    /* Premiere frontiere de 1 Gio STRICTEMENT a l'interieur (la reservation
     * peut commencer pile sur une frontiere) : toujours presente sur 3 Gio. */
    uint64_t g1 = (lo + 4 * MIO + GIO - 1) & ~(GIO - 1);
    if (g1 + 4 * MIO < hi && g1 - 2 * MIO - PAGE > lo)
        t[n++] = (struct trou){ g1 - 2 * MIO - PAGE, g1 + 3 * MIO + PAGE };
    uint64_t d = lo + 1300 * MIO + 7 * PAGE;
    t[n++] = (struct trou){ d, d + 700 * MIO + 11 * PAGE };
    for (int i = 0; i < n; i++) {
        ecris(t[i].debut, cycle);
        ecris(t[i].fin - PAGE, cycle);
        ecris(t[i].fin, cycle);
        ecris(t[i].debut - PAGE, cycle);
        if (munmap((void *)t[i].debut, t[i].fin - t[i].debut) != 0)
            echec("munmap_partiel", t[i].debut);
    }

    int gardees = 0, demontees = 0;
    for (int i = 0; i < n_ecrites; i++) {
        uint64_t a = ecrites[i];
        if (dans(a, t, n)) {
            if (!faute_en_lecture((volatile uint64_t *)a))
                echec("page_demontee_lisible", a);
            demontees++;
        } else {
            if (*(volatile uint64_t *)a != motif(a, cycle))
                echec("page_gardee_alteree", a);
            gardees++;
        }
    }
    /* Pages jamais ecrites au milieu des trous : elles aussi doivent fauter. */
    for (int i = 0; i < n; i++) {
        uint64_t m = (t[i].debut + (t[i].fin - t[i].debut) / 2) & ~(PAGE - 1);
        if (!faute_en_lecture((volatile uint64_t *)m))
            echec("milieu_trou_lisible", m);
    }
    /* Remappage au meme endroit : pages a zero. */
    for (int i = 0; i < n; i++) {
        void *r = mmap((void *)t[i].debut, t[i].fin - t[i].debut, PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED | MAP_NORESERVE, -1, 0);
        if (r != (void *)t[i].debut) {
            echec("remmap", t[i].debut);
            continue;
        }
        for (int k = 0; k < n_ecrites; k++)
            if (dans(ecrites[k], &t[i], 1) && *(volatile uint64_t *)ecrites[k] != 0)
                echec("remmap_non_nul", ecrites[k]);
    }
    if (munmap(p, taille) != 0)
        echec("munmap_total", lo);
    if (!faute_en_lecture((volatile uint64_t *)(lo + PAGE)))
        echec("apres_munmap_total_lisible", lo + PAGE);
    if (n != 3)
        echec("trous_manquants", (uint64_t)n);
    if (cycle == 0)
        printf("MUNMAP_INV mmap pages_ecrites=%d gardees=%d demontees=%d trous=%d\n",
            n_ecrites, gardees, demontees, n);
}

static void cycle_brk(int cycle)
{
    uint64_t tas = 64 * MIO;
    char *base = (char *)syscall(SYS_brk, 0);
    if ((char *)syscall(SYS_brk, base + tas) != base + tas) {
        echec("brk_grow", (uint64_t)base);
        return;
    }
    uint64_t lo = (uint64_t)base, hi = lo + tas;
    n_ecrites = 0;
    for (uint64_t b = (lo + BLOC - 1) & ~(BLOC - 1); b < hi; b += BLOC) {
        if (b - PAGE >= lo) ecris(b - PAGE, cycle);
        if (b + PAGE <= hi) ecris(b, cycle);
    }
    ecris(lo, cycle);
    ecris(hi - PAGE, cycle);
    /* Reduction a une borne NON alignee (37 Mio + 3 pages + 100 octets). */
    uint64_t borne = lo + 37 * MIO + 3 * PAGE + 100;
    uint64_t borne_page = (borne + PAGE - 1) & ~(PAGE - 1);
    if ((uint64_t)syscall(SYS_brk, (char *)borne) != borne)
        echec("brk_shrink", borne);
    for (int i = 0; i < n_ecrites; i++) {
        uint64_t a = ecrites[i];
        if (a < borne_page) {
            if (*(volatile uint64_t *)a != motif(a, cycle))
                echec("brk_page_gardee", a);
        } else if (!faute_en_lecture((volatile uint64_t *)a)) {
            echec("brk_page_retiree_lisible", a);
        }
    }
    /* Re-croissance : les pages rendues sont neuves (zero). */
    if ((char *)syscall(SYS_brk, base + tas) != base + tas)
        echec("brk_regrow", (uint64_t)base);
    for (int i = 0; i < n_ecrites; i++) {
        uint64_t a = ecrites[i];
        if (a >= borne_page && *(volatile uint64_t *)a != 0)
            echec("brk_ancien_contenu_visible", a);
    }
    syscall(SYS_brk, base);
}

int main(int argc, char **argv)
{
    int cycles = argc > 1 ? atoi(argv[1]) : 10;
    sans_fork = argc > 2 && strcmp(argv[2], "sans_fork") == 0;
    setvbuf(stdout, 0, _IOLBF, 0);

    /* Un cycle d'amorce : les tables et caches internes atteignent leur regime. */
    cycle_mmap(0);
    cycle_brk(0);
    uint64_t avant = libre();
    for (int c = 1; c <= cycles; c++) {
        cycle_mmap(c);
        cycle_brk(c);
    }
    uint64_t apres = libre();
    long long delta_pages = ((long long)avant - (long long)apres) / (long long)PAGE;
    printf("MUNMAP_INV memoire cycles=%d libre_avant_kio=%llu libre_apres_kio=%llu pages_perdues=%lld fautes_attendues_vues=%d mode=%s meme_adresse=%d\n",
        cycles, (unsigned long long)(avant >> 10), (unsigned long long)(apres >> 10), delta_pages, fautes, sans_fork ? "sans_fork" : "controle", meme_adresse);
    /* Tolerance : 64 pages (journal, caches du noyau) -- pas une fuite par cycle. */
    /* Le compte ne juge que le demontage SEUL (`sans_fork`) : en passe de
     * controle, chaque enfant laisse aussi sa trace dans le noyau. */
    if (sans_fork && !meme_adresse)
        echec("adresse_non_reutilisee", 0);
    if (sans_fork && delta_pages > 64)
        echec("cadres_perdus", (uint64_t)delta_pages);
    if (echecs) {
        printf("MUNMAP_INV_KO echecs=%d\n", echecs);
        return 1;
    }
    printf("MUNMAP_INV_OK\n");
    return 0;
}
