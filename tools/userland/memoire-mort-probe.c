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

static int zombie(void)
{
    const size_t taille = 48 * MIO;
    /* `tube` : le fils annonce qu'il tient sa memoire ; `feu` : le pere l'a
     * mesuree, le fils peut mourir. Sans ce second tube, le fils sortait
     * aussitot apres son annonce et le pere mesurait parfois un fils DEJA
     * mort (memoire deja rendue : « vivant » valait le libre initial, et le
     * banc concluait a tort que le zombie gardait sa memoire). */
    int tube[2], feu[2];
    if (pipe(tube) != 0 || pipe(feu) != 0)
        return echec("zombie", "pipe", 0);
    pid_t pid = fork();
    if (pid < 0)
        return echec("zombie", "fork", 0);
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
        _exit(0);
    }
    close(tube[1]);
    close(feu[0]);
    char c;
    if (read(tube[0], &c, 1) != 1)
        return echec("zombie", "pret", 0);
    unsigned long vivant = libre_mio();
    if (write(feu[1], &c, 1) != 1)
        return echec("zombie", "feu", 0);
    close(feu[1]);
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
        return echec("zombie", "waitpid", 0);
    unsigned long recolte = libre_mio();
    printf("MEMOIRE_MORT_ZOMBIE libre_vivant_mio=%lu libre_mort_mio=%lu "
           "libre_recolte_mio=%lu rendue_avant_recolte=%d delai_ms=%ld\n",
           vivant, apres, recolte, rendue, delai);
    if (!WIFEXITED(statut) || WEXITSTATUS(statut) != 0)
        return echec("zombie", "statut_fils", statut);
    if (!rendue)
        return echec("zombie", "memoire_gardee_par_le_zombie", (long)(apres - vivant));
    printf("MEMOIRE_MORT_OK mode=zombie\n");
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
        return zombie();
    if (strcmp(mode, "oom") == 0)
        return oom();
    return echec(mode, "mode_inconnu", 0);
}
