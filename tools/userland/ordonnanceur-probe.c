// La latence de l'interface pendant que d'autres processus calculent.
//
// C'est la seule mesure qui compte pour decider si un processus de rendu
// separe vaudra la peine. L'audit OS avait conclu : sur un cœur unique et un
// tourniquet strict, sortir le rendu d'un processus n'empeche pas une page
// lourde de rendre l'interface lente, parce que rien ne favorise l'interface.
// Deux classes d'ordonnancement ont ete ajoutees ; cette sonde dit si elles
// changent quelque chose, et de combien.
//
// Le dispositif :
//
//   * un processus **interface** se reveille toutes les 16 ms — la cadence
//     d'un rafraichissement — et note de combien il est en retard. Un reveil
//     demande a 16 ms qui arrive a 40 ms accuse 24 ms de retard, et c'est ce
//     retard-la que l'utilisateur ressent comme une saccade ;
//   * plusieurs processus **calcul** tournent sans jamais se bloquer, comme le
//     ferait une page qui met en page dix mille elements.
//
// Trois mesures, dans cet ordre, parce que chacune sert de temoin a la
// suivante :
//
//   1. l'interface seule — la latence de base, celle du timer ;
//   2. l'interface pendant le calcul, **sans** priorite ;
//   3. l'interface pendant le calcul, **avec** priorite.
//
// La troisieme doit s'approcher de la premiere. Et les processus de calcul
// doivent avoir progresse dans les trois : une priorite qui les affamerait ne
// serait pas une priorite mais une exclusion — « l'interface reste fluide »
// deviendrait « rien d'autre ne tourne ».
//
// ## Pourquoi plusieurs calculs, et pas un seul
//
// Un tourniquet a quantum d'un tick fait attendre au reveil **un quantum par
// concurrent pret**. Avec un seul concurrent, le pire cas vaut un tick — c'est
// aussi la resolution de l'horloge, donc la mesure ne peut rien distinguer.
// La premiere version de cette sonde n'employait qu'un calcul et concluait
// « la charge ne degrade pas la latence », ce qui etait vrai et sans interet :
// elle mesurait le plancher de son propre instrument.
//
// Avec huit concurrents, le pire cas attendu vaut huit ticks, soit huit fois
// la resolution. L'effet devient lisible, et son absence aussi.
//
// ## Ce que la sonde refuse de faire
//
// Conclure quand elle ne mesure rien. Si la charge ne degrade pas la latence —
// parce que la machine a plusieurs cœurs, ou parce que la resolution de
// l'horloge est trop grossiere —, elle le dit et n'affirme aucune amelioration.
// Un test qui validerait n'importe quelle implementation, y compris une qui ne
// fait rien, serait pire qu'absent.
//
//   musl-gcc -O2 -static-pie ordonnanceur-probe.c -o ordonnanceur-probe
//   (ou via ./build.sh musl)

#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int echecs = 0;
static int reussites = 0;

static void verifie(const char *nom, int condition, const char *detail)
{
    if (condition) {
        reussites++;
    } else {
        echecs++;
        printf("  - %s%s%s\n", nom, detail ? " : " : "", detail ? detail : "");
    }
}

static long long maintenant_us(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000000 + t.tv_nsec / 1000;
}

#define PERIODE_US 16000
#define REVEILS 60
#define CALCULS_MAX 16

// Ce qu'une serie de reveils apprend. Les quantiles hauts comptent plus que la
// mediane : une interface dont une trame sur vingt arrive en retard se voit,
// alors qu'une mediane parfaite ne se remarque pas.
struct latence {
    long long median;
    long long p95;
    long long p99;
    long long pire;
};

// La granularite reelle de l'horloge : le plus petit ecart non nul entre deux
// lectures consecutives. Rien de ce que la sonde mesure en dessous de cette
// valeur n'a de sens, et le dire evite de commenter du bruit.
static long long granularite_us(void)
{
    long long plus_petit = -1;
    int vus = 0;
    // Borne volontairement basse : chaque tour coute deux appels systeme, et
    // sous emulation cela se paie. Il suffit d'attraper quelques changements de
    // valeur — un par tick — pour connaitre le pas de l'horloge.
    for (int i = 0; i < 20000 && vus < 8; i++) {
        long long a = maintenant_us();
        long long b = maintenant_us();
        if (b > a) {
            vus++;
            if (plus_petit < 0 || b - a < plus_petit)
                plus_petit = b - a;
        }
        if (plus_petit == 1)
            break;
    }
    return plus_petit < 0 ? 0 : plus_petit;
}

// Le processus d'interface : se reveiller a l'heure, et mesurer de combien on
// ne l'est pas.
static void mesure_interface(struct latence *sortie)
{
    long long retards[REVEILS];
    long long attendu = maintenant_us();

    for (int i = 0; i < REVEILS; i++) {
        attendu += PERIODE_US;
        long long avant = maintenant_us();
        long long reste = attendu - avant;
        if (reste > 0) {
            struct timespec duree = {
                .tv_sec = reste / 1000000,
                .tv_nsec = (reste % 1000000) * 1000,
            };
            nanosleep(&duree, NULL);
        }
        long long reel = maintenant_us();
        retards[i] = reel > attendu ? reel - attendu : 0;
    }

    // Tri par insertion : soixante valeurs, la simplicite vaut mieux ici.
    for (int i = 1; i < REVEILS; i++) {
        long long v = retards[i];
        int j = i - 1;
        while (j >= 0 && retards[j] > v) { retards[j + 1] = retards[j]; j--; }
        retards[j + 1] = v;
    }
    sortie->median = retards[REVEILS / 2];
    sortie->p95 = retards[(REVEILS * 95) / 100];
    sortie->p99 = retards[(REVEILS * 99) / 100];
    sortie->pire = retards[REVEILS - 1];
}

static void imprime(const char *libelle, const struct latence *l)
{
    printf("  %-24s median %5lld us  p95 %6lld us  p99 %6lld us  pire %6lld us\n",
           libelle, l->median, l->p95, l->p99, l->pire);
}

// Un processus de calcul : une boucle qui ne se bloque jamais. Il ecrit son
// compteur de tours dans le tube avant de mourir, ce qui prouve qu'il a
// progresse.
static void calcule_jusqua(int tube, int secondes)
{
    long long fin = maintenant_us() + (long long)secondes * 1000000;
    volatile unsigned long long accumulateur = 0;
    unsigned long long tours = 0;
    while (maintenant_us() < fin) {
        for (int i = 0; i < 20000; i++)
            accumulateur += i * 2654435761u;
        tours++;
    }
    if (write(tube, &tours, sizeof tours) < 0)
        _exit(1);
    _exit(0);
}

// Lance `combien` calculs en fond, mesure l'interface pendant ce temps, et rend
// le total des tours effectues — c'est lui qui dit si les calculs ont avance.
static unsigned long long sous_charge(int interactif, int combien,
                                      struct latence *sortie)
{
    int tube[2];
    pid_t enfants[CALCULS_MAX];
    int nes = 0;

    if (pipe(tube) != 0)
        return 0;

    for (int i = 0; i < combien && i < CALCULS_MAX; i++) {
        pid_t enfant = fork();
        if (enfant < 0)
            break;
        if (enfant == 0) {
            close(tube[0]);
            // Le calcul reste normal dans les deux cas : ce qu'on change est la
            // priorite de **l'interface**, pas la sienne. Le degrader serait
            // une autre experience, et une moins honnete.
            calcule_jusqua(tube[1], 2);
        }
        enfants[nes++] = enfant;
    }
    close(tube[1]);

    if (interactif) {
        // `nice` negatif : le seul moyen portable de se declarer interactif.
        // Un programme ecrit pour Linux n'a rien de special a faire.
        if (setpriority(PRIO_PROCESS, 0, -5) != 0)
            printf("  (setpriority a echoue : %s)\n", strerror(errno));
    }
    mesure_interface(sortie);
    if (interactif)
        setpriority(PRIO_PROCESS, 0, 0);

    unsigned long long total = 0;
    for (int i = 0; i < nes; i++) {
        unsigned long long tours = 0;
        if (read(tube[0], &tours, sizeof tours) == (ssize_t)sizeof tours)
            total += tours;
    }
    close(tube[0]);
    for (int i = 0; i < nes; i++) {
        int statut = 0;
        waitpid(enfants[i], &statut, 0);
    }
    return total;
}

int main(int argc, char **argv)
{
    int calculs = argc > 1 ? atoi(argv[1]) : 8;
    if (calculs < 1)
        calculs = 1;
    if (calculs > CALCULS_MAX)
        calculs = CALCULS_MAX;

    printf("ordonnanceur-probe: latence de l'interface sous charge\n");

    long long grain = granularite_us();
    printf("  horloge : granularite %lld us — periode visee %d us,"
           " %d processus de calcul\n", grain, PERIODE_US, calculs);

    // 1. Le temoin : l'interface seule.
    struct latence repos;
    mesure_interface(&repos);
    imprime("au repos", &repos);

    // 2. Sous charge, sans priorite.
    struct latence normale;
    unsigned long long tours_normale = sous_charge(0, calculs, &normale);
    imprime("sous charge (normal)", &normale);
    printf("  %-24s %llu tours de calcul\n", "", tours_normale);

    // 3. Sous charge, avec priorite.
    struct latence prio;
    unsigned long long tours_prio = sous_charge(1, calculs, &prio);
    imprime("sous charge (interactif)", &prio);
    printf("  %-24s %llu tours de calcul\n", "", tours_prio);

    char detail[256];

    // Le test n'a de sens que si la charge degrade reellement la latence, et
    // qu'elle la degrade au-dela de ce que l'horloge sait distinguer. Sans
    // cela, comparer « avec » et « sans » priorite ne mesurerait que du bruit.
    long long plancher = grain > 0 ? grain * 2 : 1000;
    int charge_visible = normale.p95 > repos.p95 + plancher;

    if (!charge_visible) {
        printf("  (la charge ne degrade pas la latence de facon mesurable :\n"
               "   p95 au repos %lld us, p95 sous charge %lld us, granularite\n"
               "   de l'horloge %lld us. Soit la machine a plusieurs cœurs,\n"
               "   soit l'effet est plus fin que l'instrument. Aucune\n"
               "   conclusion n'est tiree sur la priorite.)\n",
               repos.p95, normale.p95, grain);
    } else {
        snprintf(detail, sizeof detail,
                 "p95 sans %lld / avec %lld us ; p99 sans %lld / avec %lld us ;"
                 " pire sans %lld / avec %lld us",
                 normale.p95, prio.p95, normale.p99, prio.p99,
                 normale.pire, prio.pire);
        // C'est la **queue** de la distribution qui se juge, pas la mediane :
        // une interface dont une trame sur cent arrive en retard se voit, une
        // mediane parfaite ne se remarque pas. Exiger que le p95 baisse serait
        // trop strict — la mesure validee sur Linux montre qu'a cet endroit
        // les deux courbes se croisent dans le bruit, alors que le p99 et le
        // pire cas, eux, se separent nettement.
        verifie("la priorite interactive raccourcit la queue de latence",
                prio.p99 < normale.p99 || prio.pire < normale.pire, detail);
        verifie("et n'aggrave pas le pire cas",
                prio.pire <= normale.pire, detail);
    }

    // Et surtout : le calcul doit avoir progresse dans les deux cas. Une
    // priorite qui l'affamerait ne serait pas une priorite.
    snprintf(detail, sizeof detail, "normal %llu tours, interactif %llu tours",
             tours_normale, tours_prio);
    verifie("le calcul n'est pas affame", tours_prio > 0, detail);
    if (tours_normale > 0) {
        // On tolere une forte reduction — c'est le but — mais pas
        // l'annulation : au moins un dixieme du travail doit passer.
        verifie("et il conserve une part significative du temps",
                tours_prio * 10 >= tours_normale, detail);
    }

    printf("RESULTAT : %d verification(s) en echec (%d passees)\n",
           echecs, reussites);
    return echecs == 0 ? 0 : 1;
}
