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
// ## Ce que la sonde refuse aussi : un vert que la mesure ne porte pas
//
// BOUCHAUD_SONDE_ORDO_QUALIFICATION_V1. Run 38569175760, KVM SMP1 : trois
// passes « RESULTAT : 0 », et aucune n'avait rien compare. La phase « au
// repos » -- le temoin -- etait plus lente que la phase sous charge (p95
// 1 935 contre 1 127 us) : le processeur virtuel arrete sur `hlt` recevait
// ses tics en retard (34 absences du PIT de plus de 2 ms, hors de toute
// section masquee). La sonde jugeait « charge non mesurable », sautait les
// deux comparaisons de priorite et rendait 0. Un temoin contamine relevait
// aussi le bruit admis par la seconde verification (`repos.pire`).
//
// Desormais :
//
//   * le noyau dit, pour chaque reveil en retard, quelle part vient du temps
//     qui manquait HORS de l'invite (`/proc/self/reveil`, seconde ligne,
//     BOUCHAUD_ATTRIBUTION_REVEIL_V2) et liste les reveils CONTAMINES ;
//   * la sonde apparie chacun a l'un de ses reveils et juge chaque
//     comparaison deux fois : sur TOUS ses reveils, et sur les seuls reveils
//     PROPRES. Une ligne `PREUVE` chiffree accompagne chaque comparaison ;
//   * trois verdicts : QUALIFIE_OK, QUALIFIE_ECHEC (les deux jugements
//     echouent), NON_QUALIFIANT (raison donnee : les deux jugements
//     different, une phase est trop contaminee pour etre jugee, la charge ne
//     degrade rien de mesurable, l'attribution manque). Code de sortie 0, 1,
//     2 : un NON_QUALIFIANT n'est jamais une reussite ;
//   * une mesure NON_QUALIFIANTE est refaite, au plus `tentatives` fois en
//     tout (3 par defaut), chaque tentative imprimee en entier. Un echec
//     qualifie arrete tout : il n'est jamais remesure.
//
// Les criteres eux-memes sont inchanges.
//
//   musl-gcc -O2 -static-pie ordonnanceur-probe.c -o ordonnanceur-probe
//   (ou via ./build.sh musl)
//   ordonnanceur-probe [calculs=8] [tentatives=3]

#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static long long maintenant_us(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000000 + t.tv_nsec / 1000;
}

#define PERIODE_US 16000
// BOUCHAUD_SONDE_ORDO_QUANTILES_V1
//
// C'etait 60. Avec 60 reveils, `p99 = retards[60 * 99 / 100] = retards[59]`,
// c'est-a-dire le DERNIER : le « p99 » etait le pire cas, et les deux
// verifications comparaient la meme valeur -- un seul echantillon par phase.
// Sous emulation, cet echantillon est celui ou l'hote a retire son fil au
// processeur virtuel : la sonde echouait une fois sur deux sur n'importe quel
// noyau (mesure : 5 demarrages x 5 passes, image d'avant le chantier BKL,
// 5 echecs sur 10), alors que la mediane interactive etait meilleure a
// chaque passe. Avec 300 reveils, le p99 est le troisieme pire : un vrai
// quantile, distinct du maximum, comme la sonde le supposait (sa validation
// sur Linux comparait bien deux valeurs differentes).
#define REVEILS 300
#define CALCULS_MAX 16

// Le calcul doit couvrir TOUTE la mesure : 300 reveils de 16 ms font 4,8 s.
// Il durait 2 s, pour une mesure qui en durait alors moins d'une.
#define CALCUL_SECONDES ((REVEILS * PERIODE_US) / 1000000 + 2)

// Au-dela de 5 % de reveils contamines (15 sur 300), le p95 d'une phase n'est
// plus encadre par des reveils propres : la phase ne peut pas etre jugee.
#define CONTAMINES_MAX ((REVEILS * 5) / 100)
// L'echeance du noyau est celle de la sonde arrondie a la milliseconde
// SUPERIEURE (`nanosleep` compte en millisecondes) : fenetre d'appariement.
#define APPARIE_AVANT_US 500
#define APPARIE_APRES_US 2500

// Ce qu'une serie de reveils apprend. Les quantiles hauts comptent plus que la
// mediane : une interface dont une trame sur vingt arrive en retard se voit,
// alors qu'une mediane parfaite ne se remarque pas.
struct latence {
    int n;
    long long median;
    long long p95;
    long long p99;
    long long pire;
};

// Une phase : ses reveils, ce que le noyau en dit, et ses quantiles sur TOUS
// les reveils et sur les seuls reveils PROPRES.
struct phase {
    const char *nom;
    long long attendu[REVEILS];
    long long retard[REVEILS];
    char contamine[REVEILS];
    struct latence tout;
    struct latence propre;
    // BOUCHAUD_ATTRIBUTION_REVEIL_V1 : ce que le noyau dit du PIRE reveil
    // de la serie. Lu UNE fois, apres la serie : la mesure n'en est pas
    // perturbee.
    char attribution[700];
    // BOUCHAUD_ATTRIBUTION_REVEIL_V2 : la ventilation de chaque reveil en
    // retard et la liste des reveils contamines.
    char ventilation[1800];
    int attribution_presente;
    long long contamines_noyau;
    int liste_tronquee;
    int apparies;
    int non_apparies;
    // Annonces hors de la fenetre de la serie : d'autres sommeils du meme
    // fil (dans un appel systeme avant le premier reveil mesure), qui ne
    // portent sur aucun echantillon. Comptees, imprimees, jamais appariees.
    int hors_fenetre;
    // Vide si la phase peut etre jugee ; sinon, pourquoi pas.
    char non_jugeable[160];
};

// Copie bornee ; rend 0 si `source` ne tenait pas.
static int copie(char *dest, size_t taille, const char *source)
{
    size_t n = strlen(source);
    if (n >= taille) {
        memcpy(dest, source, taille - 1);
        dest[taille - 1] = '\0';
        return 0;
    }
    memcpy(dest, source, n + 1);
    return 1;
}

// Lit `/proc/self/reveil` : le noyau y rend, puis remet a zero, ce qu'il sait
// des reveils du fil depuis la lecture precedente. Rend 0 si le fichier
// manque (hors de Bouchaud).
static int releve_attribution(struct phase *ph)
{
    char brut[2600];
    ph->attribution[0] = '\0';
    ph->ventilation[0] = '\0';
    int fd = open("/proc/self/reveil", O_RDONLY);
    if (fd < 0)
        return 0;
    ssize_t lus = 0;
    for (;;) {
        ssize_t r = read(fd, brut + lus, sizeof brut - 1 - (size_t)lus);
        if (r <= 0)
            break;
        lus += r;
        if ((size_t)lus >= sizeof brut - 1)
            break;
    }
    close(fd);
    if (lus <= 0)
        return 0;
    brut[lus] = '\0';
    char *fin = strchr(brut, '\n');
    char *seconde = NULL;
    if (fin) {
        *fin = '\0';
        seconde = fin + 1;
        char *fin2 = strchr(seconde, '\n');
        if (fin2)
            *fin2 = '\0';
    }
    copie(ph->attribution, sizeof ph->attribution, brut);
    // Une ventilation tronquee perdrait des reveils contamines : elle est
    // tenue pour absente (la phase ne sera pas jugee).
    if (seconde && strncmp(seconde, "ventilation ", 12) == 0
        && !copie(ph->ventilation, sizeof ph->ventilation, seconde + 12))
        ph->ventilation[0] = '\0';
    return 1;
}

static long long champ(const char *ligne, const char *cle)
{
    const char *p = strstr(ligne, cle);
    return p ? atoll(p + strlen(cle)) : -1;
}

// Apparie chaque reveil contamine annonce par le noyau a un reveil de la
// sonde. Un reveil annonce DANS la fenetre de la serie qui ne s'apparie a
// rien rend la phase non jugeable : la sonde ne saurait pas lequel exclure.
static void apparie(struct phase *ph)
{
    memset(ph->contamine, 0, sizeof ph->contamine);
    ph->apparies = 0;
    ph->non_apparies = 0;
    ph->hors_fenetre = 0;
    long long fenetre_debut = ph->attendu[0] - PERIODE_US;
    long long fenetre_fin = ph->attendu[REVEILS - 1] + APPARIE_APRES_US;
    ph->contamines_noyau = champ(ph->ventilation, "contamines=");
    ph->liste_tronquee = champ(ph->ventilation, "liste_tronquee=") == 1;
    const char *l = strstr(ph->ventilation, "liste=");
    if (!l)
        return;
    l += 6;
    while (*l && *l != ' ' && *l != '-') {
        long long echeance = atoll(l);
        int trouve = 0;
        if (echeance < fenetre_debut || echeance > fenetre_fin) {
            ph->hors_fenetre++;
            trouve = 1;
        }
        for (int i = 0; !trouve && i < REVEILS; i++) {
            if (echeance >= ph->attendu[i] - APPARIE_AVANT_US
                && echeance <= ph->attendu[i] + APPARIE_APRES_US) {
                if (!ph->contamine[i])
                    ph->apparies++;
                ph->contamine[i] = 1;
                trouve = 1;
                break;
            }
        }
        if (!trouve)
            ph->non_apparies++;
        const char *v = strchr(l, ',');
        if (!v)
            break;
        l = v + 1;
    }
}

static void quantiles(const long long *valeurs, int n, struct latence *sortie)
{
    long long tries[REVEILS];
    memcpy(tries, valeurs, sizeof(long long) * (size_t)n);
    // Tri par insertion : trois cents valeurs, la simplicite vaut mieux ici.
    for (int i = 1; i < n; i++) {
        long long v = tries[i];
        int j = i - 1;
        while (j >= 0 && tries[j] > v) { tries[j + 1] = tries[j]; j--; }
        tries[j + 1] = v;
    }
    sortie->n = n;
    if (n == 0) {
        sortie->median = sortie->p95 = sortie->p99 = sortie->pire = 0;
        return;
    }
    sortie->median = tries[n / 2];
    sortie->p95 = tries[(n * 95) / 100];
    sortie->p99 = tries[(n * 99) / 100];
    sortie->pire = tries[n - 1];
}

static void finalise_phase(struct phase *ph);

// Le processus d'interface : se reveiller a l'heure, et mesurer de combien on
// ne l'est pas.
static void mesure_interface(struct phase *ph)
{
    // Remise a zero : seuls les reveils de cette serie compteront.
    releve_attribution(ph);
    long long attendu = maintenant_us();

    for (int i = 0; i < REVEILS; i++) {
        attendu += PERIODE_US;
        ph->attendu[i] = attendu;
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
        ph->retard[i] = reel > attendu ? reel - attendu : 0;
    }
    ph->attribution_presente = releve_attribution(ph) && ph->ventilation[0];
    finalise_phase(ph);
}

// Ce qu'une serie de reveils mesuree permet de juger : appariement des
// reveils contamines, quantiles sur tous et sur les propres, et si la phase
// peut etre jugee. Pure (testee sur l'hote : test_ordonnanceur_verdict.cpp).
static void finalise_phase(struct phase *ph)
{
    apparie(ph);

    quantiles(ph->retard, REVEILS, &ph->tout);
    long long propres[REVEILS];
    int n = 0;
    for (int i = 0; i < REVEILS; i++)
        if (!ph->contamine[i])
            propres[n++] = ph->retard[i];
    quantiles(propres, n, &ph->propre);

    ph->non_jugeable[0] = '\0';
    if (!ph->attribution_presente)
        snprintf(ph->non_jugeable, sizeof ph->non_jugeable,
                 "%s : attribution du noyau absente (/proc/self/reveil)", ph->nom);
    else if (ph->liste_tronquee)
        snprintf(ph->non_jugeable, sizeof ph->non_jugeable,
                 "%s : %lld reveils contamines annonces, liste tronquee", ph->nom,
                 ph->contamines_noyau);
    else if (ph->apparies > CONTAMINES_MAX)
        snprintf(ph->non_jugeable, sizeof ph->non_jugeable,
                 "%s : %d reveils contamines sur %d (plus de %d)", ph->nom,
                 ph->apparies, REVEILS, CONTAMINES_MAX);
    else if (ph->non_apparies > 0)
        snprintf(ph->non_jugeable, sizeof ph->non_jugeable,
                 "%s : %d reveil(s) contamine(s) dans la serie non apparie(s)"
                 " (%d apparies, %lld annonces)",
                 ph->nom, ph->non_apparies, ph->apparies, ph->contamines_noyau);
}

static void imprime(const struct phase *ph)
{
    printf("  %-24s median %5lld us  p95 %6lld us  p99 %6lld us  pire %6lld us\n",
           ph->nom, ph->tout.median, ph->tout.p95, ph->tout.p99, ph->tout.pire);
    printf("  %-24s median %5lld us  p95 %6lld us  p99 %6lld us  pire %6lld us"
           "  [propres %d/%d, hors_fenetre=%d]\n",
           ph->nom, ph->propre.median, ph->propre.p95, ph->propre.p99, ph->propre.pire,
           ph->propre.n, REVEILS, ph->hors_fenetre);
    if (ph->attribution[0])
        printf("  attribution %s : %s\n", ph->nom, ph->attribution);
    if (ph->ventilation[0])
        printf("  ventilation %s : %s\n", ph->nom, ph->ventilation);
    if (ph->non_jugeable[0])
        printf("  phase non jugeable -- %s\n", ph->non_jugeable);
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
static unsigned long long sous_charge(int interactif, int combien, struct phase *sortie)
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
            calcule_jusqua(tube[1], CALCUL_SECONDES);
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

// ---------------------------------------------------------------------------
// Les verdicts.

enum verdict { QUALIFIE_OK, QUALIFIE_ECHEC, NON_QUALIFIANT };

static const char *nom_verdict(enum verdict v)
{
    return v == QUALIFIE_OK ? "QUALIFIE_OK" : v == QUALIFIE_ECHEC ? "QUALIFIE_ECHEC" : "NON_QUALIFIANT";
}

struct bilan {
    int ok;
    int echecs;
    int non_qualifiantes;
    char raison[260];
};

static void compte(struct bilan *b, const char *nom, enum verdict v, const char *raison)
{
    if (v == QUALIFIE_OK) {
        b->ok++;
    } else if (v == QUALIFIE_ECHEC) {
        b->echecs++;
        printf("  - %s\n", nom);
    } else {
        b->non_qualifiantes++;
        if (!b->raison[0])
            snprintf(b->raison, sizeof b->raison, "%s", raison);
    }
}

// Une comparaison jugee sur TOUS les reveils et sur les seuls reveils
// PROPRES. Les deux d'accord : verdict qualifie. En desaccord : la
// contamination fait le verdict -- non qualifiant, dans les deux sens.
static enum verdict juge(const char *nom, int tout, int propre, const char *tout_txt,
                         const char *propre_txt, const char *non_jugeable, const char *contamines,
                         char *raison, size_t taille)
{
    enum verdict v;
    raison[0] = '\0';
    if (non_jugeable && non_jugeable[0]) {
        v = NON_QUALIFIANT;
        snprintf(raison, taille, "phase non jugeable (%s)", non_jugeable);
    } else if (tout == propre) {
        v = tout ? QUALIFIE_OK : QUALIFIE_ECHEC;
    } else {
        v = NON_QUALIFIANT;
        snprintf(raison, taille, "%s : %s sur tous les reveils, %s sur les reveils propres",
                 nom, tout ? "vrai" : "faux", propre ? "vrai" : "faux");
    }
    printf("  PREUVE %s : tout[%s] = %s ; propre[%s] = %s ; contamines %s -> %s%s%s\n",
           nom, tout_txt, tout ? "vrai" : "faux", propre_txt, propre ? "vrai" : "faux",
           contamines, nom_verdict(v), raison[0] ? " : " : "", raison);
    return v;
}

static const char *premiere_non_jugeable(const struct phase *a, const struct phase *b, const struct phase *c)
{
    if (a && a->non_jugeable[0]) return a->non_jugeable;
    if (b && b->non_jugeable[0]) return b->non_jugeable;
    if (c && c->non_jugeable[0]) return c->non_jugeable;
    return "";
}

static long long granularite_us(void);
static enum verdict evalue(const struct phase *repos, const struct phase *normale,
                           const struct phase *prio, unsigned long long tours_normale,
                           unsigned long long tours_prio, long long grain, struct bilan *b);

// Une tentative entiere : trois phases, cinq comparaisons. Rend le verdict.
static enum verdict tentative(int calculs, long long grain, struct bilan *b)
{
    static struct phase repos, normale, prio;
    repos.nom = "au repos";
    normale.nom = "sous charge (normal)";
    prio.nom = "sous charge (interactif)";
    memset(b, 0, sizeof *b);

    // 1. Le temoin : l'interface seule.
    mesure_interface(&repos);
    imprime(&repos);

    // 2. Sous charge, sans priorite.
    unsigned long long tours_normale = sous_charge(0, calculs, &normale);
    imprime(&normale);
    printf("  %-24s %llu tours de calcul\n", "", tours_normale);

    // 3. Sous charge, avec priorite.
    unsigned long long tours_prio = sous_charge(1, calculs, &prio);
    imprime(&prio);
    printf("  %-24s %llu tours de calcul\n", "", tours_prio);

    return evalue(&repos, &normale, &prio, tours_normale, tours_prio, grain, b);
}

// Les cinq comparaisons d'une tentative, sur ses trois phases mesurees.
// Pure (testee sur l'hote : test_ordonnanceur_verdict.cpp).
static enum verdict evalue(const struct phase *repos_, const struct phase *normale_,
                           const struct phase *prio_, unsigned long long tours_normale,
                           unsigned long long tours_prio, long long grain, struct bilan *b)
{
    const struct phase *repos = repos_, *normale = normale_, *prio = prio_;
    memset(b, 0, sizeof *b);
    char contamines[96], raison[200], t[200], p[200];
    snprintf(contamines, sizeof contamines, "repos=%d normal=%d interactif=%d sur %d",
             repos->apparies, normale->apparies, prio->apparies, REVEILS);

    // 0. La porte. Le test n'a de sens que si la charge degrade reellement la
    // latence, au-dela de ce que l'horloge sait distinguer. Sans cela,
    // comparer « avec » et « sans » priorite ne mesurerait que du bruit --
    // et ne rien comparer n'est pas reussir.
    long long plancher = grain > 0 ? grain * 2 : 1000;
    snprintf(t, sizeof t, "p95 sous charge %lld > p95 repos %lld + plancher %lld",
             normale->tout.p95, repos->tout.p95, plancher);
    snprintf(p, sizeof p, "p95 sous charge %lld > p95 repos %lld + plancher %lld",
             normale->propre.p95, repos->propre.p95, plancher);
    enum verdict porte = juge("la charge degrade la latence (porte)",
                              normale->tout.p95 > repos->tout.p95 + plancher,
                              normale->propre.p95 > repos->propre.p95 + plancher, t, p,
                              premiere_non_jugeable(repos, normale, NULL), contamines,
                              raison, sizeof raison);
    const char *n1 = "la priorite interactive raccourcit la queue de latence";
    const char *n2 = "et n'aggrave pas le pire cas au-dela du bruit au repos";
    if (porte != QUALIFIE_OK) {
        char pourquoi[260];
        if (porte == QUALIFIE_ECHEC)
            snprintf(pourquoi, sizeof pourquoi,
                     "charge sans effet mesurable : p95 sous charge %lld <= p95 repos %lld + %lld"
                     " (tous et propres) -- la priorite n'est pas comparable",
                     normale->tout.p95, repos->tout.p95, plancher);
        else
            snprintf(pourquoi, sizeof pourquoi, "porte non qualifiee (%s)", raison);
        printf("  PREUVE %s : non jugee -> NON_QUALIFIANT : %s\n", n1, pourquoi);
        printf("  PREUVE %s : non jugee -> NON_QUALIFIANT : %s\n", n2, pourquoi);
        compte(b, n1, NON_QUALIFIANT, pourquoi);
        compte(b, n2, NON_QUALIFIANT, pourquoi);
    } else {
        // C'est la **queue** de la distribution qui se juge, pas la mediane :
        // une interface dont une trame sur cent arrive en retard se voit, une
        // mediane parfaite ne se remarque pas. Exiger que le p95 baisse serait
        // trop strict — la mesure validee sur Linux montre qu'a cet endroit
        // les deux courbes se croisent dans le bruit, alors que le p99 et le
        // pire cas, eux, se separent nettement.
        snprintf(t, sizeof t, "p99 avec %lld < sans %lld ou pire avec %lld < sans %lld",
                 prio->tout.p99, normale->tout.p99, prio->tout.pire, normale->tout.pire);
        snprintf(p, sizeof p, "p99 avec %lld < sans %lld ou pire avec %lld < sans %lld",
                 prio->propre.p99, normale->propre.p99, prio->propre.pire, normale->propre.pire);
        enum verdict v1 = juge(n1,
                               prio->tout.p99 < normale->tout.p99 || prio->tout.pire < normale->tout.pire,
                               prio->propre.p99 < normale->propre.p99 || prio->propre.pire < normale->propre.pire,
                               t, p, premiere_non_jugeable(normale, prio, NULL), contamines,
                               raison, sizeof raison);
        compte(b, n1, v1, raison);
        // BOUCHAUD_SONDE_ORDO_MARGE_REPOS_V1
        //
        // Le pire cas d'une phase est UN echantillon sur trois cents : celui
        // ou l'hote a retire son fil au processeur virtuel. Comparer deux
        // maxima isoles, c'est comparer deux tirages de ce bruit -- l'erreur
        // que BOUCHAUD_SONDE_ORDO_QUANTILES_V1 a corrigee pour le p99. Mesure,
        // endurance 38442965822, SMP1 : p95 2 482 -> 1 230 us, p99 3 144 ->
        // 1 280 us, et pourtant ECHEC sur pire 7 360 -> 8 639 us ; douze
        // passes locales donnent un pire AU REPOS de 3,9 a 7,9 ms (SMP1) et
        // jusqu'a 38 ms (SMP8). Une aggravation n'est imputable a
        // l'ordonnanceur que si elle depasse le bruit maximal observe au
        // repos dans la MEME execution ; en deca, la sonde ne sait pas.
        // Le bruit admis est celui du temoin : jugee aussi sur les seuls
        // reveils propres, la marge ne peut plus venir d'un tic perdu.
        snprintf(t, sizeof t, "pire avec %lld <= sans %lld + bruit au repos %lld",
                 prio->tout.pire, normale->tout.pire, repos->tout.pire);
        snprintf(p, sizeof p, "pire avec %lld <= sans %lld + bruit au repos %lld",
                 prio->propre.pire, normale->propre.pire, repos->propre.pire);
        enum verdict v2 = juge(n2,
                               prio->tout.pire <= normale->tout.pire + repos->tout.pire,
                               prio->propre.pire <= normale->propre.pire + repos->propre.pire,
                               t, p, premiere_non_jugeable(repos, normale, prio), contamines,
                               raison, sizeof raison);
        compte(b, n2, v2, raison);
    }

    // Et surtout : le calcul doit avoir progresse dans les deux cas. Une
    // priorite qui l'affamerait ne serait pas une priorite. Ces comptes ne
    // dependent d'aucun reveil : toujours qualifies.
    const char *n3 = "le calcul n'est pas affame";
    snprintf(t, sizeof t, "tours interactif %llu > 0", tours_prio);
    compte(b, n3, juge(n3, tours_prio > 0, tours_prio > 0, t, t, "", "sans objet", raison, sizeof raison),
           raison);
    if (tours_normale > 0) {
        // On tolere une forte reduction — c'est le but — mais pas
        // l'annulation : au moins un dixieme du travail doit passer.
        const char *n4 = "et il conserve une part significative du temps";
        snprintf(t, sizeof t, "tours interactif %llu x 10 >= normal %llu", tours_prio, tours_normale);
        int c = tours_prio * 10 >= tours_normale;
        compte(b, n4, juge(n4, c, c, t, t, "", "sans objet", raison, sizeof raison), raison);
    }

    if (b->echecs)
        return QUALIFIE_ECHEC;
    if (b->non_qualifiantes)
        return NON_QUALIFIANT;
    return QUALIFIE_OK;
}

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

int main(int argc, char **argv)
{
    int calculs = argc > 1 ? atoi(argv[1]) : 8;
    if (calculs < 1)
        calculs = 1;
    if (calculs > CALCULS_MAX)
        calculs = CALCULS_MAX;
    int tentatives = argc > 2 ? atoi(argv[2]) : 3;
    if (tentatives < 1)
        tentatives = 1;
    if (tentatives > 5)
        tentatives = 5;

    printf("ordonnanceur-probe: latence de l'interface sous charge\n");

    long long grain = granularite_us();
    printf("  horloge : granularite %lld us — periode visee %d us,"
           " %d processus de calcul, %d tentative(s) au plus\n", grain, PERIODE_US, calculs, tentatives);

    struct bilan b;
    enum verdict v = NON_QUALIFIANT;
    int k;
    for (k = 1; k <= tentatives; k++) {
        printf("  tentative %d/%d\n", k, tentatives);
        v = tentative(calculs, grain, &b);
        printf("  tentative %d/%d : %s%s%s\n", k, tentatives, nom_verdict(v),
               b.raison[0] && v == NON_QUALIFIANT ? " : " : "", v == NON_QUALIFIANT ? b.raison : "");
        // Un verdict qualifie, reussite ou echec, est definitif : seul un
        // NON_QUALIFIANT est remesure -- jamais un echec.
        if (v != NON_QUALIFIANT)
            break;
    }
    if (k > tentatives)
        k = tentatives;

    if (v == NON_QUALIFIANT) {
        printf("VERDICT_SONDE : NON_QUALIFIANT tentatives=%d raison=%s\n", k, b.raison);
        // Jamais la forme « RESULTAT : 0 » : aucun lecteur ne doit y lire une
        // reussite.
        printf("RESULTAT : NON_QUALIFIANT %d verification(s) en echec, %d non qualifiante(s) (%d passees)\n",
               b.echecs, b.non_qualifiantes, b.ok);
        return 2;
    }
    printf("VERDICT_SONDE : %s tentatives=%d\n", nom_verdict(v), k);
    printf("RESULTAT : %d verification(s) en echec (%d passees)\n", b.echecs, b.ok);
    return v == QUALIFIE_OK ? 0 : 1;
}
