/*
 * MEMOIRE-MORT-PROBE -- BOUCHAUD_ESPACE_RENDU_A_LA_MORT_V1 et
 * BOUCHAUD_OOM_CONTROLE_V1, dans le vrai noyau.
 *
 *   memoire-mort-probe zombie
 *      Un fils touche 48 Mio puis meurt. Le pere NE LE RECOLTE PAS : il
 *      attend la fin de fichier de son tube (descripteurs fermes a la mort),
 *      puis surveille `sysinfo`. La memoire doit revenir AVANT `wait4`.
 *      Avant ce lot, elle ne revenait qu'a la recolte.
 *
 *   memoire-mort-probe oom
 *      Un fils « gouffre » remplit la memoire jusqu'a ce qu'il n'en reste
 *      que quelques Mio, puis s'arrete. Le pere -- petit -- alloue alors
 *      64 Mio. C'est LUI qui faute a court de frames. Le gouffre doit mourir
 *      de SIGKILL et le pere survivre. Avant ce lot, le fautif mourait
 *      (SIGSEGV, issue=erreur_es) et le gouffre restait.
 *
 *     MEMOIRE_MORT_OK mode=...       MEMOIRE_MORT_FAIL mode=... raison=...
 */
#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <sys/mman.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysinfo.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define MIO (1024UL * 1024UL)

static unsigned long libre_mio(void)
{
    struct sysinfo s;
    if (sysinfo(&s) != 0)
        return 0;
    return (unsigned long)(s.freeram * s.mem_unit / MIO);
}

static long ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000L + t.tv_nsec / 1000000L;
}

static int echec(const char *mode, const char *raison, long v)
{
    printf("MEMOIRE_MORT_FAIL mode=%s raison=%s valeur=%ld\n", mode, raison, v);
    return 1;
}

static void touche(unsigned char *p, size_t n)
{
    for (size_t i = 0; i < n; i += 4096)
        p[i] = (unsigned char)i | 1;
}

static int zombie(int force)
{
    const char *mode = force ? "zombie-force" : "zombie";
    const size_t taille = 48 * MIO;
    /* `tube` : le fils annonce qu'il tient sa memoire ; `feu` : le pere l'a
     * mesuree, le fils peut mourir. Sans ce second tube, le fils sortait
     * aussitot apres son annonce et le pere mesurait parfois un fils DEJA
     * mort (memoire deja rendue : « vivant » valait le libre initial, et le
     * banc concluait a tort que le zombie gardait sa memoire). */
    int tube[2], feu[2];
    if (pipe(tube) != 0 || pipe(feu) != 0)
        return echec(mode, "pipe", 0);
    pid_t pid = fork();
    if (pid < 0)
        return echec(mode, "fork", 0);
    if (pid == 0) {
        close(tube[0]);
        close(feu[1]);
        unsigned char *p = malloc(taille);
        if (!p)
            _exit(2);
        touche(p, taille);
        char c = 'p';
        if (write(tube[1], &c, 1) != 1)
            _exit(3);
        if (read(feu[0], &c, 1) != 1)
            _exit(4);
        if (force) {
            volatile unsigned travail = 0;
            for (;;)
                travail++;
        }
        _exit(0);
    }
    close(tube[1]);
    close(feu[0]);
    char c;
    if (read(tube[0], &c, 1) != 1)
        return echec(mode, "pret", 0);
    unsigned long vivant = libre_mio();
    if (write(feu[1], &c, 1) != 1)
        return echec(mode, "feu", 0);
    close(feu[1]);
    if (force && kill(pid, SIGKILL) != 0)
        return echec(mode, "kill", 0);
    /* Fin de fichier : le fils est mort (ses descripteurs sont fermes a la
     * mort, avant toute recolte). */
    while (read(tube[0], &c, 1) > 0)
        ;
    long t0 = ms();
    unsigned long apres = vivant;
    while (ms() - t0 < 3000) {
        apres = libre_mio();
        if (apres >= vivant + 40)
            break;
        usleep(10000);
    }
    int rendue = apres >= vivant + 40;
    long delai = ms() - t0;
    int statut = 0;
    if (waitpid(pid, &statut, 0) != pid)
        return echec(mode, "waitpid", 0);
    unsigned long recolte = libre_mio();
    printf("MEMOIRE_MORT_ZOMBIE libre_vivant_mio=%lu libre_mort_mio=%lu "
           "libre_recolte_mio=%lu rendue_avant_recolte=%d delai_ms=%ld\n",
           vivant, apres, recolte, rendue, delai);
    if (force ? (!WIFSIGNALED(statut) || WTERMSIG(statut) != SIGKILL)
              : (!WIFEXITED(statut) || WEXITSTATUS(statut) != 0))
        return echec(mode, "statut_fils", statut);
    if (!rendue)
        return echec(mode, "memoire_gardee_par_le_zombie", (long)(apres - vivant));
    printf("MEMOIRE_MORT_OK mode=%s\n", mode);
    return 0;
}

static int emprunteur(void *opaque)
{
    int tube = *(int *)opaque;
    pid_t pid = getpid();
    if (write(tube, &pid, sizeof pid) != sizeof pid)
        return 2;
    volatile unsigned travail = 0;
    for (;;)
        travail++;
}

static int vfork_force(void)
{
    const char *mode = "vfork-force";
    int tube[2];
    if (pipe(tube) != 0)
        return echec(mode, "pipe", 0);
    pid_t preteur = fork();
    if (preteur < 0)
        return echec(mode, "fork", 0);
    if (preteur == 0) {
        close(tube[0]);
        const size_t taille = 64 * 1024;
        char *pile = malloc(taille);
        if (!pile)
            _exit(2);
        pid_t fils = clone(emprunteur, pile + taille,
                           CLONE_VM | CLONE_VFORK | SIGCHLD, &tube[1]);
        int statut;
        if (fils < 0 || waitpid(fils, &statut, 0) != fils
            || !WIFSIGNALED(statut) || WTERMSIG(statut) != SIGKILL)
            _exit(3);
        free(pile);
        _exit(0);
    }
    close(tube[1]);
    pid_t fils = 0;
    if (read(tube[0], &fils, sizeof fils) != sizeof fils || fils <= 0)
        return echec(mode, "pret", fils);
    close(tube[0]);
    if (kill(fils, SIGKILL) != 0)
        return echec(mode, "kill", fils);
    int statut = 0;
    if (waitpid(preteur, &statut, 0) != preteur || !WIFEXITED(statut)
        || WEXITSTATUS(statut))
        return echec(mode, "statut_preteur", statut);
    printf("MEMOIRE_MORT_OK mode=%s\n", mode);
    return 0;
}

static int cohorte_force(void)
{
    const char *mode = "cohorte-force";
    enum { NB = 82 };
    pid_t enfants[NB];
    int tube[2];
    if (pipe(tube) != 0)
        return echec(mode, "pipe", 0);
    for (int i = 0; i < NB; ++i) {
        pid_t fils = fork();
        if (fils < 0)
            return echec(mode, "fork", i);
        if (fils == 0) {
            close(tube[0]);
            volatile unsigned char *p = malloc(MIO);
            if (!p)
                _exit(2);
            for (size_t n = 0; n < MIO; n += 4096)
                p[n] = 1;
            if (write(tube[1], "p", 1) != 1)
                _exit(3);
            for (;;)
                pause();
        }
        enfants[i] = fils;
    }
    close(tube[1]);
    for (int i = 0; i < NB; ++i) {
        char c;
        if (read(tube[0], &c, 1) != 1 || c != 'p')
            return echec(mode, "pret", i);
    }
    close(tube[0]);
    const unsigned long vivant = libre_mio();
    for (int i = 0; i < NB; ++i)
        if (kill(enfants[i], SIGKILL) != 0)
            return echec(mode, "kill", i);
    const long t0 = ms();
    unsigned long apres = vivant;
    while (ms() - t0 < 5000) {
        apres = libre_mio();
        if (apres >= vivant + 76)
            break;
        usleep(10000);
    }
    for (int i = 0; i < NB; ++i) {
        int statut;
        if (waitpid(enfants[i], &statut, 0) != enfants[i]
            || !WIFSIGNALED(statut) || WTERMSIG(statut) != SIGKILL)
            return echec(mode, "statut", i);
    }
    printf("MEMOIRE_MORT_COHORTE enfants=%d libre_vivant_mio=%lu "
           "libre_mort_mio=%lu libre_recolte_mio=%lu\n",
           NB, vivant, apres, libre_mio());
    if (apres < vivant + 76)
        return echec(mode, "memoire_gardee", (long)apres - (long)vivant);
    printf("MEMOIRE_MORT_OK mode=%s\n", mode);
    return 0;
}

static long rss_kio(void)
{
    FILE *status = fopen("/proc/self/status", "r");
    if (!status)
        return -1;
    char ligne[128];
    long rss = -1;
    while (fgets(ligne, sizeof ligne, status))
        if (sscanf(ligne, "VmRSS: %ld kB", &rss) == 1)
            break;
    if (fclose(status) != 0)
        return -1;
    return rss;
}

struct collecte {
    pid_t pid;
    pid_t resultat;
    int statut;
    int erreur;
    atomic_int *prets;
};

static void *attend_meme_fils(void *arg)
{
    struct collecte *c = arg;
    atomic_fetch_add(c->prets, 1);
    c->resultat = waitpid(c->pid, &c->statut, 0);
    c->erreur = errno;
    return NULL;
}

static int rss_course(void)
{
    const char *mode = "rss-course";
    enum { NB = 4, TOURS = 20 };
    const size_t taille = 1024 * 1024;
    volatile unsigned char *pages = mmap(NULL, taille, PROT_READ | PROT_WRITE,
                                        MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (pages == MAP_FAILED)
        return echec(mode, "mmap", 0);
    for (size_t i = 0; i < taille; i += 4096)
        pages[i] = 1;
    for (int tour = 0; tour < TOURS; ++tour) {
        int depart[2];
        pid_t fils[NB];
        if (pipe(depart) != 0)
            return echec(mode, "pipe", tour);
        for (int n = 0; n < NB; ++n) {
            fils[n] = fork();
            if (fils[n] < 0)
                return echec(mode, "fork", n);
            if (fils[n] == 0) {
                close(depart[1]);
                char jeton;
                if (read(depart[0], &jeton, 1) != 1 || rss_kio() < 0)
                    _exit(2);
                const long avant = rss_kio();
                for (size_t i = 0; i < taille; i += 4096) {
                    if (pages[i] != 1)
                        _exit(3);
                    pages[i] = 2;
                }
                long copie = rss_kio();
                if (munmap((void *)pages, taille) != 0)
                    _exit(4);
                long apres = rss_kio();
                if (avant < 0 || copie != avant || apres != avant - (long)(taille / 1024)) {
                    printf("RSS_COURSE_FAIL tour=%d avant=%ld copie=%ld apres=%ld\n",
                           tour, avant, copie, apres);
                    _exit(5);
                }
                _exit(0);
            }
        }
        close(depart[0]);
        pthread_t threads[2];
        atomic_int prets = 0;
        struct collecte collecte[2] = {
            { .pid = fils[0], .prets = &prets },
            { .pid = fils[0], .prets = &prets },
        };
        for (int i = 0; i < 2; ++i)
            if (pthread_create(&threads[i], NULL, attend_meme_fils, &collecte[i]))
                return echec(mode, "pthread_create", i);
        while (atomic_load(&prets) != 2)
            sched_yield();
        if (write(depart[1], "xxxx", NB) != NB)
            return echec(mode, "depart", tour);
        close(depart[1]);
        int gagnants = 0, perdants = 0;
        for (int i = 0; i < 2; ++i) {
            if (pthread_join(threads[i], NULL))
                return echec(mode, "pthread_join", i);
            const struct collecte *c = &collecte[i];
            gagnants += c->resultat == fils[0] && WIFEXITED(c->statut) && !WEXITSTATUS(c->statut);
            perdants += c->resultat == -1 && c->erreur == ECHILD;
        }
        if (gagnants != 1 || perdants != 1)
            return echec(mode, "collecte_unique", tour);
        for (int n = 1; n < NB; ++n) {
            int statut;
            if (waitpid(fils[n], &statut, 0) != fils[n] || !WIFEXITED(statut) || WEXITSTATUS(statut))
                return echec(mode, "statut", tour);
        }
        for (size_t i = 0; i < taille; i += 4096)
            if (pages[i] != 1)
                return echec(mode, "parent_modifie", tour);
    }
    printf("MEMOIRE_MORT_OK mode=%s tours=%d fils=%d\n", mode, TOURS, TOURS * NB);
    return 0;
}

static int oom(void)
{
    const unsigned long marge_mio = 24;
    int tube[2];
    if (pipe(tube) != 0)
        return echec("oom", "pipe", 0);
    pid_t gouffre = fork();
    if (gouffre < 0)
        return echec("oom", "fork", 0);
    if (gouffre == 0) {
        close(tube[0]);
        unsigned long tenu = 0;
        while (libre_mio() > marge_mio) {
            unsigned char *p = malloc(4 * MIO);
            if (!p)
                break;
            touche(p, 4 * MIO);
            tenu += 4;
        }
        if (write(tube[1], &tenu, sizeof tenu) != sizeof tenu)
            _exit(3);
        for (;;)
            pause();
    }
    close(tube[1]);
    unsigned long tenu = 0;
    if (read(tube[0], &tenu, sizeof tenu) != sizeof tenu)
        return echec("oom", "gouffre_muet", 0);
    unsigned long libre_plein = libre_mio();
    /* Le pere, petit, alloue plus que ce qui reste : c'est lui qui faute a
     * court de frames. */
    const size_t taille = 64 * MIO;
    unsigned char *p = malloc(taille);
    if (!p)
        return echec("oom", "malloc_pere", 0);
    long t0 = ms();
    touche(p, taille);
    long duree = ms() - t0;
    int statut = 0;
    if (waitpid(gouffre, &statut, 0) != gouffre)
        return echec("oom", "waitpid", 0);
    printf("MEMOIRE_MORT_OOM gouffre_mio=%lu libre_plein_mio=%lu libre_fin_mio=%lu "
           "pere_touche_ms=%ld gouffre_signal=%d\n",
           tenu, libre_plein, libre_mio(), duree,
           WIFSIGNALED(statut) ? WTERMSIG(statut) : 0);
    if (!WIFSIGNALED(statut) || WTERMSIG(statut) != SIGKILL)
        return echec("oom", "gouffre_pas_tue", statut);
    printf("MEMOIRE_MORT_OK mode=oom\n");
    return 0;
}

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IOLBF, 0);
    const char *mode = argc > 1 ? argv[1] : "zombie";
    printf("MEMOIRE_MORT_BEGIN mode=%s libre_mio=%lu\n", mode, libre_mio());
    if (strcmp(mode, "zombie") == 0)
        return zombie(0);
    if (strcmp(mode, "zombie-force") == 0)
        return zombie(1);
    if (strcmp(mode, "vfork-force") == 0)
        return vfork_force();
    if (strcmp(mode, "cohorte-force") == 0)
        return cohorte_force();
    if (strcmp(mode, "rss-course") == 0)
        return rss_course();
    if (strcmp(mode, "oom") == 0)
        return oom();
    return echec(mode, "mode_inconnu", 0);
}
