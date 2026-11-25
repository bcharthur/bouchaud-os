/*
 * PAGE-ZERO-PROBE -- BOUCHAUD_PAGE_ZERO_V1.
 *
 * Une zone anonyme LUE mais jamais ecrite ne doit pas couter de frame privee :
 *   lecture   128 Mio mmap anonyme, une lecture par page : tout vaut 0, et la
 *             memoire libre baisse de moins de 8 Mio (tables de pages)
 *   ecriture  ecrire 1 Mio de pages deja lues les materialise, valeurs justes,
 *             le reste lit toujours 0
 *   fork      apres fork, le fils ecrit une page jamais ecrite : le pere y lit
 *             toujours 0
 *   munmap    la zone rendue, la memoire revient
 *
 *     PAGE_ZERO_OK              PAGE_ZERO_FAIL raison=...
 *     PAGE_ZERO_MESURE lecture_kio=... ecriture_kio=...
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/sysinfo.h>
#include <sys/wait.h>
#include <unistd.h>

#define PAGE 4096UL
#define MIO (1024UL * 1024UL)

static long libre_kio(void)
{
    struct sysinfo s;
    sysinfo(&s);
    return (long)(s.freeram * s.mem_unit / 1024);
}

static int echec(const char *r, long v)
{
    printf("PAGE_ZERO_FAIL raison=%s valeur=%ld\n", r, v);
    return 1;
}

int main(void)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    const size_t n = 128 * MIO;
    long avant = libre_kio();
    volatile unsigned char *p = mmap(NULL, n, PROT_READ | PROT_WRITE,
                                     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED)
        return echec("mmap", 0);
    unsigned long somme = 0;
    for (size_t i = 0; i < n; i += PAGE)
        somme += p[i] + p[i + PAGE - 1];
    long apres_lecture = libre_kio();
    long lecture = avant - apres_lecture;
    if (somme != 0)
        return echec("lecture_non_nulle", (long)somme);
    printf("PAGE_ZERO_ETAPE lecture=ok cout_kio=%ld\n", lecture);
    if (lecture > 8 * 1024)
        return echec("lecture_materialise", lecture);

    for (size_t i = 0; i < MIO; i += PAGE)
        p[i + 7] = (unsigned char)(i / PAGE) | 1;
    long ecriture = apres_lecture - libre_kio();
    for (size_t i = 0; i < MIO; i += PAGE)
        if (p[i + 7] != (unsigned char)((i / PAGE) | 1) || p[i] != 0)
            return echec("ecriture_fausse", (long)i);
    for (size_t i = MIO; i < n; i += 64 * PAGE)
        if (p[i + 7] != 0)
            return echec("voisine_modifiee", (long)i);
    printf("PAGE_ZERO_ETAPE ecriture=ok cout_kio=%ld\n", ecriture);
    if (ecriture < 256)
        return echec("ecriture_sans_frame", ecriture);

    size_t cible = 64 * MIO;
    pid_t pid = fork();
    if (pid == 0) {
        p[cible] = 0x5A;
        _exit(p[cible] == 0x5A ? 0 : 1);
    }
    int statut = 0;
    waitpid(pid, &statut, 0);
    if (!WIFEXITED(statut) || WEXITSTATUS(statut) != 0)
        return echec("fils", statut);
    if (p[cible] != 0)
        return echec("pere_voit_le_fils", p[cible]);
    printf("PAGE_ZERO_ETAPE fork=ok\n");

    munmap((void *)p, n);
    long fin = libre_kio();
    printf("PAGE_ZERO_MESURE lecture_kio=%ld ecriture_kio=%ld rendu_kio=%ld\n",
           lecture, ecriture, fin - avant);
    if (avant - fin > 4 * 1024)
        return echec("munmap_garde", avant - fin);
    printf("PAGE_ZERO_ETAPE munmap=ok\n");
    printf("PAGE_ZERO_OK\n");
    return 0;
}
