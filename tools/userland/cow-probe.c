/*
 * COW-PROBE -- BOUCHAUD_COW_V1 : fork en copie-a-l'ecriture, dans le vrai
 * noyau, avec la glibc.
 *
 * Epreuves (chacune imprime COW_ETAPE nom=ok, ou COW_FAIL raison=...) :
 *   pere_ecrit      le pere ecrit apres fork ; le fils voit l'ancien contenu
 *   fils_ecrit      le fils ecrit ; le pere garde le sien
 *   noyau_ecrit     read() d'un tube DANS une page COW du pere : le noyau
 *                   ecrit pour le pere seul ; le fils garde l'ancien contenu
 *   partage         MAP_SHARED|MAP_ANONYMOUS reste partage apres fork
 *   pere_meurt      le pere intermediaire meurt avant le fils, qui ecrit
 *                   ensuite dans les pages heritees
 *   fils_meurt      le fils meurt ; le pere ecrit ensuite (promotion)
 *   futex           apres fork, un fil du pere attend un futex dans une page
 *                   COW ; un autre fil ecrit le mot (copie) puis reveille :
 *                   l'attendant DOIT se reveiller (cle de futex stable)
 *   rafale          N forks dont chaque fils ecrit des pages au hasard et
 *                   verifie le reste
 *   memoire         un pere de 64 Mio forke : la memoire libre ne baisse
 *                   presque pas (partage), et revient apres la recolte
 *
 * Usage : cow-probe [N_RAFALE] [EPREUVE]   (EPREUVE seule si donnee : temoin)
 */
#define _GNU_SOURCE
#include <errno.h>
#include <linux/futex.h>
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/sysinfo.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define PAGE 4096
#define MIO (1024UL * 1024UL)

static int echec(const char *raison, long v)
{
    printf("COW_FAIL raison=%s valeur=%ld\n", raison, v);
    fflush(stdout);
    return 1;
}

static unsigned long libre_kio(void)
{
    struct sysinfo s;
    sysinfo(&s);
    return (unsigned long)(s.freeram * s.mem_unit / 1024);
}

static int attend_fils(pid_t pid)
{
    int statut = 0;
    if (waitpid(pid, &statut, 0) != pid)
        return -1;
    return WIFEXITED(statut) ? WEXITSTATUS(statut) : 128 + WTERMSIG(statut);
}

static void remplit(unsigned char *p, size_t n, unsigned char graine)
{
    for (size_t i = 0; i < n; i++)
        p[i] = (unsigned char)(graine + i * 7);
}

static int verifie(const unsigned char *p, size_t n, unsigned char graine)
{
    for (size_t i = 0; i < n; i++)
        if (p[i] != (unsigned char)(graine + i * 7))
            return 0;
    return 1;
}

/* Barriere pere/fils par tubes. */
static void signale(int fd) { char c = 'x'; if (write(fd, &c, 1) != 1) _exit(90); }
static void attend(int fd) { char c; if (read(fd, &c, 1) != 1) _exit(91); }

static int pere_ou_fils_ecrit(int pere_ecrit)
{
    const size_t n = 16 * PAGE;
    unsigned char *p = malloc(n);
    remplit(p, n, 11);
    int vers_fils[2], vers_pere[2];
    if (pipe(vers_fils) || pipe(vers_pere))
        return echec("pipe", errno);
    pid_t pid = fork();
    if (pid < 0)
        return echec("fork", errno);
    if (pid == 0) {
        if (pere_ecrit) {
            attend(vers_fils[0]);
            _exit(verifie(p, n, 11) ? 0 : 1);
        }
        remplit(p, n, 99);
        signale(vers_pere[1]);
        _exit(verifie(p, n, 99) ? 0 : 2);
    }
    if (pere_ecrit) {
        remplit(p, n, 55);
        signale(vers_fils[1]);
    } else {
        attend(vers_pere[0]);
    }
    int code = attend_fils(pid);
    if (code != 0)
        return echec(pere_ecrit ? "pere_ecrit_fils_voit_la_modif" : "fils_ecrit_statut", code);
    if (!verifie(p, n, pere_ecrit ? 55 : 11))
        return echec(pere_ecrit ? "pere_ecrit_perdu" : "fils_ecrit_vu_par_le_pere", 0);
    free(p);
    return 0;
}

static int noyau_ecrit(void)
{
    unsigned char *p = mmap(NULL, 4 * PAGE, PROT_READ | PROT_WRITE,
                            MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED)
        return echec("mmap", errno);
    remplit(p, 4 * PAGE, 3);
    int donnees[2], vers_fils[2];
    if (pipe(donnees) || pipe(vers_fils))
        return echec("pipe", errno);
    pid_t pid = fork();
    if (pid == 0) {
        attend(vers_fils[0]);
        _exit(verifie(p, 4 * PAGE, 3) ? 0 : 1);
    }
    /* Le noyau ecrit dans la page COW du pere, AU MILIEU d'une page. */
    unsigned char motif[200];
    memset(motif, 0xA5, sizeof motif);
    if (write(donnees[1], motif, sizeof motif) != (ssize_t)sizeof motif)
        return echec("write_tube", errno);
    if (read(donnees[0], p + PAGE + 100, sizeof motif) != (ssize_t)sizeof motif)
        return echec("read_dans_page_cow", errno);
    signale(vers_fils[1]);
    int code = attend_fils(pid);
    if (code != 0)
        return echec("noyau_ecrit_vu_par_le_fils", code);
    for (size_t i = 0; i < sizeof motif; i++)
        if (p[PAGE + 100 + i] != 0xA5)
            return echec("noyau_ecrit_perdu", (long)i);
    munmap(p, 4 * PAGE);
    return 0;
}

static int partage(void)
{
    volatile uint32_t *p = mmap(NULL, PAGE, PROT_READ | PROT_WRITE,
                                MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED)
        return echec("mmap_partage", errno);
    p[0] = 1;
    pid_t pid = fork();
    if (pid == 0) {
        p[0] = 42;
        _exit(0);
    }
    if (attend_fils(pid) != 0)
        return echec("partage_statut", 0);
    if (p[0] != 42)
        return echec("map_shared_n_est_plus_partage", p[0]);
    munmap((void *)p, PAGE);
    return 0;
}

static int pere_meurt(void)
{
    int rapport[2];
    if (pipe(rapport))
        return echec("pipe", errno);
    pid_t inter = fork();
    if (inter == 0) {
        const size_t n = 8 * PAGE;
        unsigned char *p = malloc(n);
        remplit(p, n, 21);
        pid_t petit = fork();
        if (petit == 0) {
            usleep(150000); /* le pere intermediaire est mort entre-temps */
            int ok = verifie(p, n, 21);
            remplit(p, n, 77);
            ok = ok && verifie(p, n, 77);
            char c = ok ? 'o' : 'k';
            if (write(rapport[1], &c, 1) != 1)
                _exit(3);
            _exit(0);
        }
        remplit(p, n, 33); /* le pere ecrit puis meurt sans attendre */
        _exit(verifie(p, n, 33) ? 0 : 1);
    }
    close(rapport[1]);
    if (attend_fils(inter) != 0)
        return echec("pere_meurt_intermediaire", 0);
    char c = 0;
    if (read(rapport[0], &c, 1) != 1)
        return echec("pere_meurt_petit_fils_muet", 0);
    close(rapport[0]);
    return c == 'o' ? 0 : echec("pere_meurt_contenu", c);
}

static int fils_meurt(void)
{
    const size_t n = 8 * PAGE;
    unsigned char *p = malloc(n);
    remplit(p, n, 5);
    pid_t pid = fork();
    if (pid == 0)
        _exit(verifie(p, n, 5) ? 0 : 1);
    if (attend_fils(pid) != 0)
        return echec("fils_meurt_statut", 0);
    remplit(p, n, 6); /* seul proprietaire : promotion sans copie */
    if (!verifie(p, n, 6))
        return echec("fils_meurt_promotion", 0);
    free(p);
    return 0;
}

/* --- futex apres fork --------------------------------------------------- */
static uint32_t *mot_futex;
static volatile int reveille;

static long futex(uint32_t *u, int op, uint32_t v)
{
    return syscall(SYS_futex, u, op, v, NULL, NULL, 0);
}

static void *attendant(void *arg)
{
    (void)arg;
    while (__atomic_load_n(mot_futex, __ATOMIC_SEQ_CST) == 0)
        futex(mot_futex, FUTEX_WAIT_PRIVATE, 0);
    reveille = 1;
    return NULL;
}

static int futex_apres_fork(void)
{
    /* Le mot vit dans sa propre page, partagee en COW par le fork. */
    mot_futex = mmap(NULL, PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (mot_futex == MAP_FAILED)
        return echec("mmap_futex", errno);
    mot_futex[0] = 0;
    mot_futex[100] = 1; /* la page est residente */
    pid_t pid = fork();
    if (pid == 0) {
        usleep(400000);
        _exit(mot_futex[0] == 0 ? 0 : 1);
    }
    pthread_t t;
    pthread_create(&t, NULL, attendant, NULL);
    usleep(100000); /* l'attendant est gare sur l'ANCIENNE frame */
    __atomic_store_n(mot_futex, 1, __ATOMIC_SEQ_CST); /* copie a l'ecriture */
    futex(mot_futex, FUTEX_WAKE_PRIVATE, 1);
    for (int i = 0; i < 200 && !reveille; i++)
        usleep(10000);
    if (!reveille)
        return echec("futex_reveil_perdu_apres_copie", 0);
    pthread_join(t, NULL);
    if (attend_fils(pid) != 0)
        return echec("futex_fils_voit_l_ecriture", 0);
    munmap(mot_futex, PAGE);
    return 0;
}

static int rafale(int n)
{
    const size_t taille = 32 * PAGE;
    unsigned char *p = malloc(taille);
    remplit(p, taille, 9);
    for (int i = 0; i < n; i++) {
        pid_t pid = fork();
        if (pid < 0)
            return echec("rafale_fork", errno);
        if (pid == 0) {
            unsigned int graine = (unsigned int)i * 2654435761u;
            int touchees[32] = {0};
            for (int k = 0; k < 6; k++) {
                int pg = (int)((graine >> (k * 4)) % 32);
                touchees[pg] = 1;
                memset(p + pg * PAGE, 0xEE, PAGE);
            }
            for (int pg = 0; pg < 32; pg++) {
                const unsigned char *q = p + pg * PAGE;
                for (int j = 0; j < PAGE; j++) {
                    unsigned char attendu = touchees[pg] ? 0xEE : (unsigned char)(9 + (pg * PAGE + j) * 7);
                    if (q[j] != attendu)
                        _exit(1);
                }
            }
            _exit(0);
        }
        int code = attend_fils(pid);
        if (code != 0)
            return echec("rafale_fils", i);
        if (!verifie(p, taille, 9))
            return echec("rafale_pere_modifie", i);
    }
    free(p);
    return 0;
}

static int memoire(long *delta_fork_kio, long *rendu_kio)
{
    const size_t n = 64 * MIO;
    unsigned char *p = malloc(n);
    for (size_t i = 0; i < n; i += PAGE)
        p[i] = (unsigned char)i | 1;
    unsigned long avant = libre_kio();
    int vers_pere[2], vers_fils[2];
    if (pipe(vers_pere) || pipe(vers_fils))
        return echec("pipe", errno);
    pid_t pid = fork();
    if (pid == 0) {
        signale(vers_pere[1]);
        attend(vers_fils[0]);
        _exit(p[PAGE] == (unsigned char)(PAGE | 1) ? 0 : 1);
    }
    attend(vers_pere[0]);
    unsigned long pendant = libre_kio();
    signale(vers_fils[1]);
    if (attend_fils(pid) != 0)
        return echec("memoire_fils", 0);
    usleep(50000);
    unsigned long apres = libre_kio();
    *delta_fork_kio = (long)avant - (long)pendant;
    *rendu_kio = (long)apres - (long)pendant;
    free(p);
    /* Un fork recopiant aurait pris >= 64 Mio. */
    if (*delta_fork_kio > 16 * 1024)
        return echec("memoire_fork_recopie", *delta_fork_kio);
    return 0;
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    int n = argc > 1 ? atoi(argv[1]) : 50;
    unsigned long debut = libre_kio();
    printf("COW_BEGIN libre_kio=%lu\n", debut);
    const char *seule = argc > 2 ? argv[2] : NULL;
#define ETAPE(nom, appel)                              \
    do {                                               \
        if (!seule || strcmp(seule, nom) == 0) {       \
            if (appel)                                 \
                return 1;                              \
            printf("COW_ETAPE %s=ok\n", nom);          \
        }                                              \
    } while (0)
    ETAPE("pere_ecrit", pere_ou_fils_ecrit(1));
    ETAPE("fils_ecrit", pere_ou_fils_ecrit(0));
    ETAPE("noyau_ecrit", noyau_ecrit());
    ETAPE("partage", partage());
    ETAPE("pere_meurt", pere_meurt());
    ETAPE("fils_meurt", fils_meurt());
    ETAPE("futex", futex_apres_fork());
    long t0 = 0;
    struct timespec a, b;
    clock_gettime(CLOCK_MONOTONIC, &a);
    ETAPE("rafale", rafale(n));
    clock_gettime(CLOCK_MONOTONIC, &b);
    t0 = (b.tv_sec - a.tv_sec) * 1000000L + (b.tv_nsec - a.tv_nsec) / 1000;
    long delta = 0, rendu = 0;
    ETAPE("memoire", memoire(&delta, &rendu));
    usleep(100000);
    unsigned long fin = libre_kio();
    printf("COW_MESURE rafale_n=%d rafale_us_par_fork=%ld fork_64mio_cout_kio=%ld "
           "rendu_apres_recolte_kio=%ld libre_debut_kio=%lu libre_fin_kio=%lu\n",
           n, t0 / (n ? n : 1), delta, rendu, debut, fin);
    printf("COW_OK\n");
    return 0;
}
