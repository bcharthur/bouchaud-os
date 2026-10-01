// scheduler-ng-banc : la ligne de base de l'ordonnanceur, mesuree depuis ring 3.
//
// BOUCHAUD_SCHEDULER_NG_BANC_V1
//
// Chantier Scheduler NG, phase 0. Rien n'est change dans le noyau avant que
// ce banc ait mesure l'existant : chaque phase suivante se juge contre ses
// chiffres, et les seuils de la phase 13 en derivent.
//
// Quatre sections, une ligne `SNG v=1 sec=...` par mesure (cle=valeur, sans
// espace dans les valeurs) -- `tools/ci/scheduler_ng_baseline.py` les agrege :
//
//   A  latence interactive : une tache se reveille toutes les 10 ms et note
//      son retard, au repos puis sous 1x et 2x autant de calculs que de
//      coeurs, sans puis avec la classe interactive (nice -5) ;
//   B  equite CPU : 2, 4, 8, 16 calculs identiques ; tours de chacun, indice
//      de Jain, rapport min/max, et le plus long trou (temps ou un calcul
//      pret n'a pas tourne) ;
//   C  charge mixte : interface + calculs + ping-pong par tube + fork/exit
//      en continu ;
//   D  cycle de vie : fork/exit/wait4, exec, mort etrangere (exit_group d'un
//      frere endormi dans nanosleep, futex, read), kill d'un endormi, racine
//      qui meurt avant ses descendants, ping-pong futex.
//
// Ce que le banc EXIGE (invariants absolus, pas des seuils) : tout fils est
// recolte avant son echeance, aucun fil d'un processus mort ne s'execute
// apres sa recolte, aucun reveil futex n'est perdu, tout calcul progresse.
// Ce qu'il MESURE seulement (seuils derives plus tard) : latences, equite.
//
// Une echeance depassee ne fige jamais le banc : il compte `perdus`, le dit,
// et continue. Un banc qui se bloque sur le defaut qu'il cherche ne mesure
// rien.
//
//   musl-gcc -O2 -static-pie -pthread scheduler-ng-banc.c -o scheduler-ng-banc
//
// Usage : scheduler-ng-banc [sections] (defaut ABCD)

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define CALCULS_MAX 16
#define ECHEANCE_RECOLTE_US 10000000LL

static int echecs = 0;
static long perdus_total = 0;
static long ressuscites_total = 0;
static long reveils_perdus_total = 0;

static long long maintenant_us(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000000 + t.tv_nsec / 1000;
}

static void dort_us(long long us)
{
    if (us <= 0)
        return;
    struct timespec d = { .tv_sec = us / 1000000, .tv_nsec = (us % 1000000) * 1000 };
    while (nanosleep(&d, &d) != 0 && errno == EINTR) {
    }
}

static int compare_ll(const void *a, const void *b)
{
    long long x = *(const long long *)a, y = *(const long long *)b;
    return (x > y) - (x < y);
}

// Quantile sur un tableau TRIE : indice `n * q / 100`, borne au dernier.
static long long quantile(const long long *trie, int n, int q)
{
    if (n <= 0)
        return 0;
    int i = (int)((long long)n * q / 100);
    return trie[i >= n ? n - 1 : i];
}

// Le nombre de coeurs EN LIGNE, lu dans /sys (plage Linux « 0-N »). Le banc
// publie aussi ce que rend `sysconf`, qui passe par sched_getaffinity sous
// musl : les deux doivent concorder, et un ecart est une mesure en soi --
// c'est la taille que choisira tout pool de threads.
static int coeurs_sysfs(void)
{
    FILE *f = fopen("/sys/devices/system/cpu/online", "r");
    if (!f)
        return 0;
    int premier = 0, dernier = -1;
    int n = fscanf(f, "%d-%d", &premier, &dernier);
    fclose(f);
    if (n == 1)
        return 1;
    if (n == 2 && dernier >= premier)
        return dernier - premier + 1;
    return 0;
}

static int coeurs_sysconf(void)
{
    long n = sysconf(_SC_NPROCESSORS_ONLN);
    return n < 1 ? 1 : (int)n;
}

static int nb_coeurs(void)
{
    int n = coeurs_sysfs();
    return n > 0 ? n : coeurs_sysconf();
}

// Recolte `pid` avant l'echeance, sans jamais se bloquer indefiniment.
// Rend 1 et le statut si le fils est recolte, 0 s'il est PERDU.
static int recolte(pid_t pid, int *statut, long long echeance_us)
{
    long long debut = maintenant_us();
    long long fin = debut + echeance_us;
    for (;;) {
        pid_t r = waitpid(pid, statut, WNOHANG);
        if (r == pid)
            return 1;
        if (r < 0 && errno != EINTR)
            return 0;
        long long t = maintenant_us();
        if (t >= fin)
            return 0;
        // Les 5 premieres ms : ceder plutot que dormir, sans quoi la mesure
        // serait celle du pas de nanosleep (un tick) et non celle de la mort.
        if (t - debut < 5000)
            sched_yield();
        else
            dort_us(1000);
    }
}

// ---------------------------------------------------------------------------
// Calculs : boucle qui ne se bloque jamais, compte ses tours et son plus long
// trou -- l'intervalle maximal entre deux tours consecutifs, c'est-a-dire le
// plus long moment ou ce calcul PRET n'a pas tourne.
// ---------------------------------------------------------------------------

struct bilan_calcul {
    unsigned long long tours;
    long long trou_max_us;
};

static void calcule(int tube, long long duree_us)
{
    long long debut = maintenant_us();
    long long fin = debut + duree_us;
    long long precedent = debut;
    volatile unsigned long long acc = 0;
    struct bilan_calcul b = { 0, 0 };
    for (;;) {
        for (int i = 0; i < 2000; i++)
            acc += (unsigned long long)i * 2654435761u;
        b.tours++;
        long long t = maintenant_us();
        if (t - precedent > b.trou_max_us)
            b.trou_max_us = t - precedent;
        precedent = t;
        if (t >= fin)
            break;
    }
    if (write(tube, &b, sizeof b) != (ssize_t)sizeof b)
        _exit(1);
    _exit(0);
}

struct groupe {
    pid_t pids[CALCULS_MAX];
    int nes;
    int tube[2];
};

static int lance_calculs(struct groupe *g, int combien, long long duree_us)
{
    g->nes = 0;
    if (pipe(g->tube) != 0)
        return -1;
    for (int i = 0; i < combien && i < CALCULS_MAX; i++) {
        pid_t p = fork();
        if (p < 0)
            break;
        if (p == 0) {
            close(g->tube[0]);
            calcule(g->tube[1], duree_us);
        }
        g->pids[g->nes++] = p;
    }
    close(g->tube[1]);
    return g->nes;
}

struct resume_calculs {
    int n;
    unsigned long long min, max, total;
    long jain_milli;      // (sum x)^2 / (n * sum x^2), x1000
    long ratio_milli;     // min / max, x1000
    long long trou_max_us;
    long long trou_median_us;
    int perdus;
};

static void recolte_calculs(struct groupe *g, struct resume_calculs *r)
{
    struct bilan_calcul b[CALCULS_MAX];
    long long trous[CALCULS_MAX];
    int lus = 0;
    for (int i = 0; i < g->nes; i++) {
        if (read(g->tube[0], &b[lus], sizeof b[lus]) == (ssize_t)sizeof b[lus])
            lus++;
    }
    close(g->tube[0]);
    memset(r, 0, sizeof *r);
    r->n = lus;
    double somme = 0, carres = 0;
    for (int i = 0; i < lus; i++) {
        unsigned long long x = b[i].tours;
        if (i == 0 || x < r->min)
            r->min = x;
        if (x > r->max)
            r->max = x;
        r->total += x;
        somme += (double)x;
        carres += (double)x * (double)x;
        trous[i] = b[i].trou_max_us;
        if (b[i].trou_max_us > r->trou_max_us)
            r->trou_max_us = b[i].trou_max_us;
    }
    if (lus > 0 && carres > 0)
        r->jain_milli = (long)(somme * somme / ((double)lus * carres) * 1000.0);
    if (r->max > 0)
        r->ratio_milli = (long)(r->min * 1000 / r->max);
    qsort(trous, lus, sizeof trous[0], compare_ll);
    r->trou_median_us = lus ? trous[lus / 2] : 0;
    for (int i = 0; i < g->nes; i++) {
        int st = 0;
        if (!recolte(g->pids[i], &st, ECHEANCE_RECOLTE_US))
            r->perdus++;
    }
    if (lus != g->nes) {
        printf("SNG v=1 sec=X anomalie=bilans_manquants lus=%d attendus=%d\n", lus, g->nes);
        echecs++;
    }
    if (r->min == 0 && lus > 0) {
        printf("SNG v=1 sec=X anomalie=calcul_affame\n");
        echecs++;
    }
    perdus_total += r->perdus;
}

// ---------------------------------------------------------------------------
// A. Latence interactive
// ---------------------------------------------------------------------------

#define PERIODE_US 10000
#define REVEILS_A 200

struct latence {
    int n;
    long long p50, p95, p99, max;
};

static void mesure_reveils(struct latence *l, int reveils, long long periode_us)
{
    long long *retards = calloc((size_t)reveils, sizeof *retards);
    if (!retards) {
        memset(l, 0, sizeof *l);
        return;
    }
    long long attendu = maintenant_us();
    for (int i = 0; i < reveils; i++) {
        attendu += periode_us;
        dort_us(attendu - maintenant_us());
        long long reel = maintenant_us();
        retards[i] = reel > attendu ? reel - attendu : 0;
        // Rattrape l'horloge sans accumuler la dette : un retard ne doit pas
        // rendre la periode suivante plus courte.
        if (reel > attendu)
            attendu = reel;
    }
    qsort(retards, reveils, sizeof *retards, compare_ll);
    l->n = reveils;
    l->p50 = quantile(retards, reveils, 50);
    l->p95 = quantile(retards, reveils, 95);
    l->p99 = quantile(retards, reveils, 99);
    l->max = retards[reveils - 1];
    free(retards);
}

static void section_a(int coeurs)
{
    struct latence l;
    mesure_reveils(&l, REVEILS_A, PERIODE_US);
    printf("SNG v=1 sec=A cas=repos calculs=0 classe=normale n=%d p50_us=%lld p95_us=%lld p99_us=%lld max_us=%lld\n",
           l.n, l.p50, l.p95, l.p99, l.max);
    int charges[2] = { coeurs, 2 * coeurs };
    for (int c = 0; c < 2; c++) {
        int n = charges[c] > CALCULS_MAX ? CALCULS_MAX : charges[c];
        for (int interactif = 0; interactif <= 1; interactif++) {
            struct groupe g;
            long long duree = (long long)REVEILS_A * PERIODE_US + 1500000;
            if (lance_calculs(&g, n, duree) < 0) {
                echecs++;
                continue;
            }
            if (interactif)
                setpriority(PRIO_PROCESS, 0, -5);
            mesure_reveils(&l, REVEILS_A, PERIODE_US);
            if (interactif)
                setpriority(PRIO_PROCESS, 0, 0);
            struct resume_calculs r;
            recolte_calculs(&g, &r);
            printf("SNG v=1 sec=A cas=charge calculs=%d classe=%s n=%d p50_us=%lld p95_us=%lld p99_us=%lld max_us=%lld calcul_tours=%llu calcul_jain_milli=%ld perdus=%d\n",
                   n, interactif ? "interactive" : "normale", l.n, l.p50, l.p95, l.p99, l.max,
                   r.total, r.jain_milli, r.perdus);
        }
    }
}

// ---------------------------------------------------------------------------
// B. Equite
// ---------------------------------------------------------------------------

static void section_b(void)
{
    int tailles[4] = { 2, 4, 8, 16 };
    for (int t = 0; t < 4; t++) {
        struct groupe g;
        long long duree = 2000000;
        long long debut = maintenant_us();
        if (lance_calculs(&g, tailles[t], duree) < 0) {
            echecs++;
            continue;
        }
        struct resume_calculs r;
        recolte_calculs(&g, &r);
        long long ecoule = maintenant_us() - debut;
        printf("SNG v=1 sec=B calculs=%d duree_ms=%lld recus=%d tours_min=%llu tours_max=%llu tours_total=%llu jain_milli=%ld ratio_min_max_milli=%ld trou_max_us=%lld trou_median_us=%lld perdus=%d\n",
               tailles[t], ecoule / 1000, r.n, r.min, r.max, r.total, r.jain_milli,
               r.ratio_milli, r.trou_max_us, r.trou_median_us, r.perdus);
    }
}

// ---------------------------------------------------------------------------
// C. Charge mixte
// ---------------------------------------------------------------------------

#define PINGPONG 300

static void pingpong_fils(int lire, int ecrire)
{
    char c;
    for (;;) {
        ssize_t n = read(lire, &c, 1);
        if (n <= 0)
            _exit(0);
        if (write(ecrire, &c, 1) != 1)
            _exit(1);
    }
}

static void churn_fils(int tube_bilan, long long duree_us)
{
    long long fin = maintenant_us() + duree_us;
    unsigned long n = 0, perdus = 0;
    while (maintenant_us() < fin) {
        pid_t p = fork();
        if (p < 0)
            continue;
        if (p == 0)
            _exit(3);
        int st = 0;
        if (!recolte(p, &st, ECHEANCE_RECOLTE_US) || !WIFEXITED(st) || WEXITSTATUS(st) != 3)
            perdus++;
        n++;
    }
    unsigned long bilan[2] = { n, perdus };
    if (write(tube_bilan, bilan, sizeof bilan) != (ssize_t)sizeof bilan)
        _exit(1);
    _exit(0);
}

static void section_c(int coeurs)
{
    int n = 2 * coeurs > CALCULS_MAX ? CALCULS_MAX : 2 * coeurs;
    long long duree = 5000000;
    struct groupe g;
    if (lance_calculs(&g, n, duree) < 0) {
        echecs++;
        return;
    }
    int aller[2], retour[2], bilan[2];
    if (pipe(aller) || pipe(retour) || pipe(bilan)) {
        echecs++;
        return;
    }
    pid_t pp = fork();
    if (pp == 0) {
        close(aller[1]);
        close(retour[0]);
        pingpong_fils(aller[0], retour[1]);
    }
    close(aller[0]);
    close(retour[1]);
    pid_t churn = fork();
    if (churn == 0) {
        // Ne pas tenir les extremites du ping-pong : sa fin de fichier ne
        // doit dependre que du banc.
        close(aller[1]);
        close(retour[0]);
        close(bilan[0]);
        churn_fils(bilan[1], duree - 1000000);
    }
    close(bilan[1]);

    // L'interface, interactive, pendant que tout le reste tourne.
    setpriority(PRIO_PROCESS, 0, -5);
    struct latence l;
    mesure_reveils(&l, 150, PERIODE_US);
    // Le ping-pong : un aller-retour = deux reveils par tube.
    long long rtt[PINGPONG];
    int faits = 0;
    for (int i = 0; i < PINGPONG; i++) {
        char c = 'p';
        long long t0 = maintenant_us();
        if (write(aller[1], &c, 1) != 1 || read(retour[0], &c, 1) != 1)
            break;
        rtt[faits++] = maintenant_us() - t0;
    }
    setpriority(PRIO_PROCESS, 0, 0);
    close(aller[1]);
    close(retour[0]);
    qsort(rtt, faits, sizeof rtt[0], compare_ll);

    unsigned long churn_bilan[2] = { 0, 0 };
    if (read(bilan[0], churn_bilan, sizeof churn_bilan) != (ssize_t)sizeof churn_bilan)
        echecs++;
    close(bilan[0]);
    int st = 0, perdus = 0, perdu_pingpong = 0, perdu_churn = 0;
    if (!recolte(pp, &st, ECHEANCE_RECOLTE_US))
        perdu_pingpong = 1;
    if (!recolte(churn, &st, ECHEANCE_RECOLTE_US))
        perdu_churn = 1;
    perdus = perdu_pingpong + perdu_churn;
    struct resume_calculs r;
    recolte_calculs(&g, &r);
    perdus_total += perdus + (long)churn_bilan[1];
    if (faits != PINGPONG)
        echecs++;
    printf("SNG v=1 sec=C calculs=%d reveil_p50_us=%lld reveil_p99_us=%lld reveil_max_us=%lld pingpong=%d rtt_p50_us=%lld rtt_p99_us=%lld rtt_max_us=%lld churn_fork_exit=%lu churn_perdus=%lu calcul_jain_milli=%ld calcul_trou_max_us=%lld perdus=%d perdu_pingpong=%d perdu_churn=%d perdus_calculs=%d\n",
           n, l.p50, l.p99, l.max, faits, quantile(rtt, faits, 50), quantile(rtt, faits, 99),
           faits ? rtt[faits - 1] : 0, churn_bilan[0], churn_bilan[1], r.jain_milli,
           r.trou_max_us, perdus + r.perdus, perdu_pingpong, perdu_churn, r.perdus);
}

// ---------------------------------------------------------------------------
// D. Cycle de vie
// ---------------------------------------------------------------------------

static void imprime_cycle(const char *cas, int n, int ok, int perdus, long long *durees, int nd,
                          const char *extra)
{
    qsort(durees, nd, sizeof durees[0], compare_ll);
    printf("SNG v=1 sec=D cas=%s n=%d ok=%d perdus=%d p50_us=%lld p99_us=%lld max_us=%lld%s%s\n",
           cas, n, ok, perdus, quantile(durees, nd, 50), quantile(durees, nd, 99),
           nd ? durees[nd - 1] : 0, extra ? " " : "", extra ? extra : "");
    if (ok != n)
        echecs++;
    perdus_total += perdus;
}

static void cycle_fork_exit(int n)
{
    long long d[256];
    int ok = 0, perdus = 0, nd = 0;
    for (int i = 0; i < n && i < 256; i++) {
        long long t0 = maintenant_us();
        pid_t p = fork();
        if (p < 0)
            continue;
        if (p == 0)
            _exit(i & 0x7f);
        int st = 0;
        if (!recolte(p, &st, ECHEANCE_RECOLTE_US)) {
            perdus++;
            continue;
        }
        d[nd++] = maintenant_us() - t0;
        if (WIFEXITED(st) && WEXITSTATUS(st) == (i & 0x7f))
            ok++;
    }
    imprime_cycle("fork-exit-wait4", n, ok, perdus, d, nd, NULL);
}

static void cycle_exec(const char *moi, int n)
{
    long long d[64];
    int ok = 0, perdus = 0, nd = 0;
    for (int i = 0; i < n && i < 64; i++) {
        long long t0 = maintenant_us();
        pid_t p = fork();
        if (p < 0)
            continue;
        if (p == 0) {
            char *argv[] = { (char *)moi, "--sortie-exec", NULL };
            execv(moi, argv);
            _exit(127);
        }
        int st = 0;
        if (!recolte(p, &st, ECHEANCE_RECOLTE_US)) {
            perdus++;
            continue;
        }
        d[nd++] = maintenant_us() - t0;
        if (WIFEXITED(st) && WEXITSTATUS(st) == 42)
            ok++;
    }
    imprime_cycle("fork-exec-exit", n, ok, perdus, d, nd, NULL);
}

// Mort etrangere : un processus de FILS fils d'execution endormis -- dans
// nanosleep, futex ou read -- dont l'un appelle exit_group. Les autres sont
// tues PAR LUI, depuis un autre coeur ou le meme. Chaque fil incremente son
// compteur dans une memoire partagee avec le banc a chaque reveil : apres la
// recolte, plus aucun compteur ne doit bouger. Un compteur qui bouge est un
// fil d'un processus mort qui s'execute encore -- un zombie ressuscite.

enum attente { PAR_NANOSLEEP, PAR_FUTEX, PAR_READ };

#define FILS_MORT 4

struct partage {
    volatile unsigned long compteurs[FILS_MORT];
    volatile int futex_mot;
    // Instant de l'exit_group, ecrit par le fil qui tue son groupe : la
    // duree mesuree est celle de la mort, pas celle de l'endormissement.
    volatile long long t_mort_us;
};

struct arg_fil {
    struct partage *p;
    int indice;
    enum attente mode;
    int tube_lecture;
};

static void *fil_endormi(void *arg)
{
    struct arg_fil *a = arg;
    for (;;) {
        switch (a->mode) {
        case PAR_NANOSLEEP:
            dort_us(1000);
            break;
        case PAR_FUTEX: {
            struct timespec ts = { 0, 2000000 };
            syscall(SYS_futex, &a->p->futex_mot, 0 /* FUTEX_WAIT */, 0, &ts, NULL, 0);
            break;
        }
        case PAR_READ: {
            char c;
            if (read(a->tube_lecture, &c, 1) <= 0)
                dort_us(1000);
            break;
        }
        }
        __atomic_add_fetch(&a->p->compteurs[a->indice], 1, __ATOMIC_RELAXED);
    }
    return NULL;
}

static const char *nom_attente(enum attente m)
{
    return m == PAR_NANOSLEEP ? "nanosleep" : m == PAR_FUTEX ? "futex" : "read";
}

static void cycle_mort_etrangere(struct partage *p, enum attente mode, int n)
{
    long long d[64];
    int ok = 0, perdus = 0, nd = 0, ressuscites = 0;
    for (int i = 0; i < n && i < 64; i++) {
        memset((void *)p, 0, sizeof *p);
        int tube[2];
        if (pipe(tube) != 0)
            continue;
        pid_t pid = fork();
        if (pid < 0)
            continue;
        if (pid == 0) {
            close(tube[1]);
            pthread_t fils[FILS_MORT];
            struct arg_fil args[FILS_MORT];
            for (int k = 1; k < FILS_MORT; k++) {
                args[k] = (struct arg_fil){ p, k, mode, tube[0] };
                pthread_create(&fils[k], NULL, fil_endormi, &args[k]);
            }
            // Laisser les fils s'endormir, puis tuer le groupe depuis ce fil.
            dort_us(2000 + (i % 5) * 700);
            __atomic_add_fetch(&p->compteurs[0], 1, __ATOMIC_RELAXED);
            p->t_mort_us = maintenant_us();
            _exit(5); // exit_group
        }
        close(tube[0]);
        int st = 0;
        if (!recolte(pid, &st, ECHEANCE_RECOLTE_US)) {
            perdus++;
            close(tube[1]);
            continue;
        }
        long long recolte_us = maintenant_us();
        d[nd++] = p->t_mort_us ? recolte_us - p->t_mort_us : 0;
        if (WIFEXITED(st) && WEXITSTATUS(st) == 5)
            ok++;
        unsigned long avant[FILS_MORT];
        for (int k = 0; k < FILS_MORT; k++)
            avant[k] = p->compteurs[k];
        // Reveiller toute attente encore possible : un fil mort n'en profite
        // pas, un fil ressuscite si.
        p->futex_mot = 1;
        syscall(SYS_futex, &p->futex_mot, 1 /* FUTEX_WAKE */, 64, NULL, NULL, 0);
        if (write(tube[1], "xxxx", 4) < 0) {
        }
        dort_us(30000);
        for (int k = 0; k < FILS_MORT; k++)
            if (p->compteurs[k] != avant[k])
                ressuscites++;
        close(tube[1]);
    }
    char extra[64];
    snprintf(extra, sizeof extra, "ressuscites=%d", ressuscites);
    char cas[48];
    snprintf(cas, sizeof cas, "mort-etrangere-%s", nom_attente(mode));
    imprime_cycle(cas, n, ok, perdus, d, nd, extra);
    ressuscites_total += ressuscites;
    if (ressuscites)
        echecs++;
}

static void cycle_kill_endormi(int n)
{
    long long d[64];
    int ok = 0, perdus = 0, nd = 0;
    for (int i = 0; i < n && i < 64; i++) {
        pid_t p = fork();
        if (p < 0)
            continue;
        if (p == 0) {
            for (;;)
                dort_us(1000);
        }
        dort_us(2000 + (i % 3) * 1000);
        long long t0 = maintenant_us();
        kill(p, SIGKILL);
        int st = 0;
        if (!recolte(p, &st, ECHEANCE_RECOLTE_US)) {
            perdus++;
            continue;
        }
        d[nd++] = maintenant_us() - t0;
        // Bouchaud termine par exit_group(128 + sig) ; Linux rend WIFSIGNALED.
        if ((WIFSIGNALED(st) && WTERMSIG(st) == SIGKILL) ||
            (WIFEXITED(st) && WEXITSTATUS(st) == 128 + SIGKILL))
            ok++;
    }
    imprime_cycle("kill-endormi", n, ok, perdus, d, nd, NULL);
}

// Racine qui meurt avant ses descendants : le banc lit un tube dont chaque
// petit-fils tient l'extremite d'ecriture ; la fin de fichier n'arrive que
// quand TOUS sont morts -- orphelins, reparentes, dans une nouvelle session.
static void cycle_racine(int n)
{
    long long d[64];
    int ok = 0, perdus = 0, nd = 0;
    for (int i = 0; i < n && i < 64; i++) {
        int tube[2];
        if (pipe(tube) != 0)
            continue;
        long long t0 = maintenant_us();
        pid_t racine = fork();
        if (racine < 0)
            continue;
        if (racine == 0) {
            close(tube[0]);
            setsid();
            for (int k = 0; k < 3; k++) {
                if (fork() == 0) {
                    dort_us(5000 + k * 3000);
                    _exit(0);
                }
            }
            _exit(0);
        }
        close(tube[1]);
        int st = 0;
        int recoltee = recolte(racine, &st, ECHEANCE_RECOLTE_US);
        // Les petits-fils meurent en 5 a 11 ms : 3 s suffisent largement, et
        // une fin de fichier qui ne vient jamais ne doit pas couter 10 s.
        // EOF sans bloquer indefiniment : lecture non bloquante sondee.
        fcntl(tube[0], F_SETFL, fcntl(tube[0], F_GETFL) | O_NONBLOCK);
        long long fin = maintenant_us() + 3000000;
        int eof = 0;
        while (maintenant_us() < fin) {
            char c;
            ssize_t r = read(tube[0], &c, 1);
            if (r == 0) {
                eof = 1;
                break;
            }
            if (r < 0 && errno != EAGAIN && errno != EINTR)
                break;
            dort_us(1000);
        }
        close(tube[0]);
        if (!recoltee || !eof) {
            perdus++;
            continue;
        }
        d[nd++] = maintenant_us() - t0;
        ok++;
    }
    imprime_cycle("racine-avant-descendants", n, ok, perdus, d, nd, NULL);
}

// Fin de fichier AVANT la recolte : un fils qui tient l'ecriture d'un tube
// meurt ; le pere doit lire la fin de fichier SANS avoir appele wait4. POSIX
// ferme les descripteurs a la mort (exit), pas a la recolte : un pere qui
// attend la fin d'un tube avant de recolter ne doit pas se bloquer. Mesure :
// fin de fichier vue avant recolte (ok), ou seulement apres (`apres_recolte`).
static void cycle_eof_avant_recolte(int n)
{
    long long d[64];
    int ok = 0, perdus = 0, nd = 0, apres_recolte = 0;
    for (int i = 0; i < n && i < 64; i++) {
        int tube[2];
        if (pipe(tube) != 0)
            continue;
        pid_t p = fork();
        if (p < 0)
            continue;
        if (p == 0) {
            close(tube[0]);
            dort_us(2000);
            _exit(0);
        }
        close(tube[1]);
        fcntl(tube[0], F_SETFL, fcntl(tube[0], F_GETFL) | O_NONBLOCK);
        long long t0 = maintenant_us();
        long long fin = t0 + 2000000;
        int eof = 0;
        while (maintenant_us() < fin) {
            char c;
            ssize_t r = read(tube[0], &c, 1);
            if (r == 0) {
                eof = 1;
                break;
            }
            dort_us(1000);
        }
        if (eof) {
            d[nd++] = maintenant_us() - t0;
            ok++;
        }
        int st = 0;
        if (!recolte(p, &st, ECHEANCE_RECOLTE_US))
            perdus++;
        if (!eof) {
            char c;
            if (read(tube[0], &c, 1) == 0)
                apres_recolte++;
        }
        close(tube[0]);
    }
    char extra[48];
    snprintf(extra, sizeof extra, "eof_apres_recolte=%d", apres_recolte);
    imprime_cycle("eof-avant-recolte", n, ok, perdus, d, nd, extra);
}

// Ping-pong futex entre deux fils : chaque echange est un reveil. Une attente
// bornee a 1 s qui expire est un reveil PERDU.
struct pingfutex {
    volatile int tour;
    int n;
    long perdus;
};

static void *ping_futex_fil(void *arg)
{
    struct pingfutex *pf = arg;
    for (int i = 0; i < pf->n; i++) {
        while (__atomic_load_n(&pf->tour, __ATOMIC_ACQUIRE) != 1) {
            struct timespec ts = { 1, 0 };
            long r = syscall(SYS_futex, &pf->tour, 0, 0, &ts, NULL, 0);
            if (r != 0 && errno == ETIMEDOUT && __atomic_load_n(&pf->tour, __ATOMIC_ACQUIRE) != 1)
                __atomic_add_fetch(&pf->perdus, 1, __ATOMIC_RELAXED);
        }
        __atomic_store_n(&pf->tour, 0, __ATOMIC_RELEASE);
        syscall(SYS_futex, &pf->tour, 1, 1, NULL, NULL, 0);
    }
    return NULL;
}

static void cycle_futex(int n)
{
    struct pingfutex pf = { 0, n, 0 };
    pthread_t f;
    long long d[1];
    long long t0 = maintenant_us();
    if (pthread_create(&f, NULL, ping_futex_fil, &pf) != 0) {
        echecs++;
        return;
    }
    for (int i = 0; i < n; i++) {
        __atomic_store_n(&pf.tour, 1, __ATOMIC_RELEASE);
        syscall(SYS_futex, &pf.tour, 1, 1, NULL, NULL, 0);
        while (__atomic_load_n(&pf.tour, __ATOMIC_ACQUIRE) != 0) {
            struct timespec ts = { 1, 0 };
            long r = syscall(SYS_futex, &pf.tour, 0, 1, &ts, NULL, 0);
            if (r != 0 && errno == ETIMEDOUT && __atomic_load_n(&pf.tour, __ATOMIC_ACQUIRE) != 0)
                __atomic_add_fetch(&pf.perdus, 1, __ATOMIC_RELAXED);
        }
    }
    pthread_join(f, NULL);
    d[0] = (maintenant_us() - t0) / (n > 0 ? n : 1);
    char extra[64];
    snprintf(extra, sizeof extra, "reveils_perdus=%ld echange_moyen_us=%lld", pf.perdus, d[0]);
    imprime_cycle("futex-pingpong", n, n, 0, d, 1, extra);
    reveils_perdus_total += pf.perdus;
    if (pf.perdus)
        echecs++;
}

static void section_d(const char *moi, struct partage *p)
{
    cycle_fork_exit(100);
    if (moi[0] == '/')
        cycle_exec(moi, 20);
    else
        printf("SNG v=1 sec=D cas=fork-exec-exit saute=chemin_relatif\n");
    cycle_mort_etrangere(p, PAR_NANOSLEEP, 30);
    cycle_mort_etrangere(p, PAR_FUTEX, 30);
    cycle_mort_etrangere(p, PAR_READ, 30);
    cycle_kill_endormi(20);
    cycle_eof_avant_recolte(10);
    cycle_racine(5);
    cycle_futex(500);
}

// 1 si une ecriture du fils dans `p` est vue par le pere.
static int verifie_partage(struct partage *p)
{
    p->t_mort_us = 0;
    pid_t f = fork();
    if (f < 0)
        return 0;
    if (f == 0) {
        p->t_mort_us = 0x5a5a;
        _exit(0);
    }
    int st = 0;
    if (!recolte(f, &st, ECHEANCE_RECOLTE_US))
        return 0;
    int ok = p->t_mort_us == 0x5a5a;
    p->t_mort_us = 0;
    return ok;
}

int main(int argc, char **argv)
{
    if (argc > 1 && strcmp(argv[1], "--sortie-exec") == 0)
        return 42;
    const char *sections = argc > 1 ? argv[1] : "ABCD";
    int coeurs = nb_coeurs();
    setvbuf(stdout, NULL, _IOLBF, 0);
    // Les tubes du banc survivent parfois a leur lecteur (mort etrangere) :
    // une ecriture doit rendre EPIPE, pas tuer le banc.
    signal(SIGPIPE, SIG_IGN);

    struct partage *p = mmap(NULL, 4096, PROT_READ | PROT_WRITE,
                             MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) {
        int fd = (int)syscall(SYS_memfd_create, "sng-partage", 1);
        if (fd < 0 || ftruncate(fd, 4096) != 0)
            return 2;
        p = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        if (p == MAP_FAILED)
            return 2;
    }

    // Le detecteur de resurrection repose sur cette page partagee : verifier
    // qu'un fils y ecrit bien ce que le pere lit, sinon le detecteur est
    // aveugle et doit le dire. Repli memfd si MAP_ANONYMOUS ne partage pas.
    int partage = verifie_partage(p);
    if (!partage) {
        int fd = (int)syscall(SYS_memfd_create, "sng-partage", 1);
        if (fd >= 0 && ftruncate(fd, 4096) == 0) {
            struct partage *q = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
            if (q != MAP_FAILED && verifie_partage(q)) {
                p = q;
                partage = 2;
            }
        }
    }
    long long debut = maintenant_us();
    printf("SNG v=1 sec=DEBUT coeurs=%d coeurs_sysfs=%d coeurs_sysconf=%d sections=%s partage_fork=%d\n",
           coeurs, coeurs_sysfs(), coeurs_sysconf(), sections, partage);
    if (!partage) {
        printf("SNG v=1 sec=X anomalie=partage_fork_absent\n");
        echecs++;
    }
    if (strchr(sections, 'A'))
        section_a(coeurs);
    if (strchr(sections, 'B'))
        section_b();
    if (strchr(sections, 'C'))
        section_c(coeurs);
    if (strchr(sections, 'D'))
        section_d(argv[0], p);
    printf("SNG v=1 sec=FIN coeurs=%d duree_ms=%lld echecs=%d perdus=%ld ressuscites=%ld reveils_perdus=%ld\n",
           coeurs, (maintenant_us() - debut) / 1000, echecs, perdus_total, ressuscites_total,
           reveils_perdus_total);
    printf("%s\n", echecs == 0 ? "SCHEDULER_NG_BANC_OK" : "SCHEDULER_NG_BANC_FAIL");
    return echecs == 0 ? 0 : 1;
}
