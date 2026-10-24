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
//   * le noyau range, a l'ENTREE de chaque sommeil de la sonde, son echeance
//     et les comptes cumules de son fil (`/proc/self/fil`,
//     BOUCHAUD_PREUVE_HOTE_V1) ; la sonde les relit UNE fois apres la serie.
//     Deux entrees consecutives encadrent un reveil sur TOUTE sa fenetre
//     (sommeil, election, retour en ring 3, lecture d'horloge) : combien
//     l'hote l'a retarde -- PREUVE a l'appui, le temps vole publie par KVM --,
//     combien d'absences de battement restent inexpliquees, combien le noyau
//     a masque, combien le fil a attendu pret. (Relire apres chaque reveil
//     ajoutait trois appels systeme -- trois points de preemption -- entre
//     l'horloge et le sommeil : la phase normale passait de 4 a 45-76 ms de
//     p95 sous TCG SMP1, la phase interactive ne bougeait pas. L'instrument
//     changeait ce qu'il mesurait, dans le sens qui facilite le vert.)
//   * la sonde juge chaque comparaison deux fois : sur TOUS ses reveils, et
//     sur les reveils PROPRES. Sous KVM (`preuve_hote=kvm-steal`), un reveil
//     n'est ecarte que si l'hote PROUVE l'avoir retarde de plus d'une
//     milliseconde ; un retard inexplique reste compte contre le noyau. Sans
//     preuve de l'hote (TCG), rien ne peut etre prouve externe : les propres
//     sont les reveils sans retard inexplique, et un desaccord des deux
//     jugements ne qualifie rien. Une ligne `PREUVE` chiffree accompagne
//     chaque comparaison ;
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
#include <sys/syscall.h>
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

// Au-dela de 5 % de reveils ecartes (15 sur 300), le p95 d'une phase n'est
// plus encadre par des reveils propres : la phase ne peut pas etre jugee.
#define CONTAMINES_MAX ((REVEILS * 5) / 100)
// Une part au-dela d'une milliseconde ecarte (ou signale) un reveil.
#define SEUIL_NS 1000000LL
// Les sommeils que le noyau peut ranger (SOMMEILS_ENREGISTRES = 512), et les
// absences de son journal (JOURNAL_ABSENCES = 256).
#define SOMMEILS_MAX 512
#define ABSENCES_MAX 256

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

// Les comptes cumules du fil, tels que `/proc/self/fil` les rend.
struct compte_fil {
    int preuve_hote; // 1 : kvm-steal ; 0 : absente
    long long externe_ns;
    long long inexplique_ns;
    long long masque_ns;
    long long pret_ns;
    long long incompletes;
    long long vol_total_ns;
    long long vol_coeurs;
    long long a_ns;
};

// Un sommeil range par le noyau : son echeance et les comptes du fil a son
// entree.
struct sommeil_noyau {
    long long echeance_us;
    long long entree_us;
    long long externe_ns;
    long long inexplique_ns;
    long long masque_ns;
    long long pret_ns;
};

// Une phase : ses reveils, ce que le noyau dit de CHACUN, et ses quantiles
// sur TOUS les reveils et sur les seuls reveils PROPRES.
struct phase {
    const char *nom;
    long long attendu[REVEILS];
    long long retard[REVEILS];
    // BOUCHAUD_PREUVE_HOTE_V1 : les sommeils ranges par le noyau, les comptes
    // a la fin de la serie, et par reveil la difference des comptes entre
    // l'entree de son sommeil et celle du suivant (ns).
    struct sommeil_noyau sommeils[SOMMEILS_MAX];
    int n_sommeils;
    long long sommeils_perdus;
    // Les absences du coeur de la sonde closes pendant qu'elle le tenait :
    // instant de fin, cause (E prouvee, M masquee, I inexpliquee), part (ns).
    long long absence_fin_us[ABSENCES_MAX];
    char absence_cause[ABSENCES_MAX];
    long long absence_part_ns[ABSENCES_MAX];
    int n_absences;
    int absences_incompletes;
    // Le reveil i a-t-il dormi (un sommeil range lui revient) ?
    char dormi[REVEILS];
    struct compte_fil final;
    // Ecart entre sommeils ranges et reveils dormis (0 attendu).
    int non_apparies;
    int sans_sommeil;
    long long externe[REVEILS];
    long long inexplique[REVEILS];
    long long masque[REVEILS];
    long long pret[REVEILS];
    // Ecarte : prouve retarde par l'hote (ou, sans preuve, inexplique).
    char contamine[REVEILS];
    struct latence tout;
    struct latence propre;
    // BOUCHAUD_ATTRIBUTION_REVEIL_V1 : ce que le noyau dit du PIRE reveil
    // de la serie. Lu UNE fois, apres la serie : la mesure n'en est pas
    // perturbee. 1 000 octets : la ligne tient jusqu'a ~640 octets depuis
    // BOUCHAUD_REPRISE_DECOUPEE_V1 (trois champs en fin de ligne, que la
    // troncature a 700 aurait emportes les premiers).
    char attribution[1000];
    // BOUCHAUD_ATTRIBUTION_REVEIL_V2 : la ventilation de la serie (resume).
    char ventilation[1800];
    int attribution_presente;
    // Les comptes du fil ont ete lus apres chaque reveil.
    int comptes_presents;
    int preuve_hote;
    long long incompletes;
    // Reveils prouves retardes par l'hote, et retardes sans explication.
    int prouves;
    int inexpliques;
    int ecartes;
    // Le vol de l'hote sur toute la serie, en pour mille du temps des coeurs.
    long long vol_pour_mille;
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

// Lit `/proc/self/fil` : les comptes CUMULES du fil, puis, si `ph`, les
// sommeils que le noyau a ranges depuis l'ouverture precedente. L'ouverture
// rearme l'enregistreur. Rend 0 si le fichier manque (hors de Bouchaud).
static char brut_fil[40000];

static int lis_fil(struct compte_fil *c, struct phase *ph)
{
    int fd = open("/proc/self/fil", O_RDONLY);
    if (fd < 0)
        return 0;
    ssize_t lus = 0;
    for (;;) {
        ssize_t r = read(fd, brut_fil + lus, sizeof brut_fil - 1 - (size_t)lus);
        if (r <= 0)
            break;
        lus += r;
        if ((size_t)lus >= sizeof brut_fil - 1)
            break;
    }
    close(fd);
    if (lus <= 0)
        return 0;
    brut_fil[lus] = '\0';
    if (strncmp(brut_fil, "fil ", 4) != 0)
        return 0;
    char *seconde = strchr(brut_fil, '\n');
    if (!seconde)
        return 0;
    *seconde++ = '\0';
    c->preuve_hote = strstr(brut_fil, "preuve_hote=kvm-steal") != NULL;
    c->externe_ns = champ(brut_fil, "externe_ns=");
    c->inexplique_ns = champ(brut_fil, "inexplique_ns=");
    c->masque_ns = champ(brut_fil, "masque_ns=");
    c->pret_ns = champ(brut_fil, "pret_ns=");
    c->incompletes = champ(brut_fil, "incompletes=");
    c->vol_total_ns = champ(brut_fil, "vol_total_ns=");
    c->vol_coeurs = champ(brut_fil, "vol_coeurs=");
    c->a_ns = champ(brut_fil, "a_ns=");
    if (c->externe_ns < 0 || c->inexplique_ns < 0 || c->masque_ns < 0 || c->pret_ns < 0 || c->a_ns <= 0)
        return 0;
    if (!ph)
        return 1;
    if (strncmp(seconde, "sommeils n=", 11) != 0)
        return 0;
    long long annonces = champ(seconde, "n=");
    ph->sommeils_perdus = champ(seconde, "perdus=");
    ph->n_sommeils = 0;
    char *l = strchr(seconde, '\n');
    while (l && *++l && strncmp(l, "absences ", 9) != 0 && ph->n_sommeils < SOMMEILS_MAX) {
        struct sommeil_noyau *z = &ph->sommeils[ph->n_sommeils];
        if (sscanf(l, "%lld %lld %lld %lld %lld %lld", &z->echeance_us, &z->entree_us, &z->externe_ns,
                   &z->inexplique_ns, &z->masque_ns, &z->pret_ns) != 6)
            break;
        ph->n_sommeils++;
        l = strchr(l, '\n');
    }
    // Une liste tronquee (tampon, ou ligne illisible) ne doit pas passer
    // pour complete.
    if (ph->n_sommeils != annonces || ph->sommeils_perdus < 0 || !l || strncmp(l, "absences n=", 11) != 0)
        return 0;
    long long absences = champ(l, "n=");
    ph->absences_incompletes = champ(l, "incomplet=") != 0;
    ph->n_absences = 0;
    l = strchr(l, '\n');
    while (l && *++l && ph->n_absences < ABSENCES_MAX) {
        char cause = 0;
        if (sscanf(l, "%lld %c %lld", &ph->absence_fin_us[ph->n_absences], &cause,
                   &ph->absence_part_ns[ph->n_absences]) != 3)
            break;
        ph->absence_cause[ph->n_absences++] = cause;
        l = strchr(l, '\n');
    }
    return ph->n_absences == absences;
}

// Repartit les comptes entre les reveils (pure ; testee sur l'hote). Les
// sommeils ranges reviennent, DANS L'ORDRE, aux reveils qui ont dormi : la
// sonde sait lesquels (`dormi`), et compter suffit -- l'echeance du noyau,
// derivee des tics, n'est pas dans l'horloge de la sonde quand des tics se
// perdent. L'intervalle d'un reveil va de l'entree de son sommeil a celle du
// suivant (ou a la fin de la serie) : les cumuls y avancent de ses parts
// livraison et election, et les absences closes dedans, pendant que la sonde
// tenait son coeur, s'y ajoutent selon leur cause. Un reveil qui n'a pas
// dormi (deja en retard) tombe dans l'intervalle du precedent et le partage.
// Autant de sommeils que de reveils dormis, ou rien n'est reparti.
static void repartit(struct phase *ph)
{
    memset(ph->externe, 0, sizeof ph->externe);
    memset(ph->inexplique, 0, sizeof ph->inexplique);
    memset(ph->masque, 0, sizeof ph->masque);
    memset(ph->pret, 0, sizeof ph->pret);
    ph->non_apparies = ph->sans_sommeil = 0;
    int dormis = 0;
    for (int i = 0; i < REVEILS; i++)
        dormis += ph->dormi[i] != 0;
    if (dormis != ph->n_sommeils) {
        // Sommeils en trop ou en moins : on ne sait pas lequel revient a qui.
        ph->non_apparies = ph->n_sommeils > dormis ? ph->n_sommeils - dormis : dormis - ph->n_sommeils;
        return;
    }
    int k = -1;
    for (int i = 0; i < REVEILS; i++) {
        if (ph->dormi[i])
            k++;
        else
            ph->sans_sommeil++;
        if (k < 0)
            continue;
        const struct sommeil_noyau *a = &ph->sommeils[k];
        long long be, bi, bm, bp, fin_us;
        if (k + 1 < ph->n_sommeils) {
            const struct sommeil_noyau *b = &ph->sommeils[k + 1];
            be = b->externe_ns; bi = b->inexplique_ns; bm = b->masque_ns; bp = b->pret_ns;
            fin_us = b->entree_us;
        } else {
            be = ph->final.externe_ns; bi = ph->final.inexplique_ns;
            bm = ph->final.masque_ns; bp = ph->final.pret_ns;
            fin_us = ph->final.a_ns / 1000;
        }
        ph->externe[i] = be - a->externe_ns;
        ph->inexplique[i] = bi - a->inexplique_ns;
        ph->masque[i] = bm - a->masque_ns;
        ph->pret[i] = bp - a->pret_ns;
        for (int z = 0; z < ph->n_absences; z++) {
            if (ph->absence_fin_us[z] < a->entree_us || ph->absence_fin_us[z] >= fin_us)
                continue;
            long long part = ph->absence_part_ns[z];
            if (ph->absence_cause[z] == 'E')
                ph->externe[i] += part;
            else if (ph->absence_cause[z] == 'M')
                ph->masque[i] += part;
            else
                ph->inexplique[i] += part;
        }
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
    // Remise a zero : seuls les reveils de cette serie compteront. La
    // lecture de `/proc/self/fil` arme l'enregistreur des sommeils.
    releve_attribution(ph);
    struct compte_fil depart;
    memset(&depart, 0, sizeof depart);
    memset(&ph->final, 0, sizeof ph->final);
    int comptes = lis_fil(&depart, NULL);
    long long attendu = maintenant_us();

    for (int i = 0; i < REVEILS; i++) {
        attendu += PERIODE_US;
        ph->attendu[i] = attendu;
        long long avant = maintenant_us();
        long long reste = attendu - avant;
        ph->dormi[i] = reste > 0;
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
    // BOUCHAUD_PREUVE_HOTE_V1 : les comptes de fin et les sommeils ranges,
    // lus APRES la serie : rien ne s'est ajoute entre deux reveils.
    comptes = comptes && lis_fil(&ph->final, ph) && ph->n_sommeils > 0;
    ph->comptes_presents = comptes;
    ph->preuve_hote = comptes ? depart.preuve_hote : 0;
    ph->incompletes = comptes ? ph->final.incompletes - depart.incompletes : 0;
    ph->vol_pour_mille = 0;
    if (comptes && ph->final.vol_coeurs > 0 && ph->final.a_ns > depart.a_ns)
        ph->vol_pour_mille = (ph->final.vol_total_ns - depart.vol_total_ns) * 1000
                             / ((ph->final.a_ns - depart.a_ns) * ph->final.vol_coeurs);
    if (comptes)
        repartit(ph);
    ph->attribution_presente = releve_attribution(ph) && ph->ventilation[0];
    finalise_phase(ph);
}

// Ce qu'une serie de reveils mesuree permet de juger : les reveils a
// ecarter, les quantiles sur tous et sur les propres, et si la phase peut
// etre jugee. Pure (testee sur l'hote : test_ordonnanceur_verdict.cpp).
static void finalise_phase(struct phase *ph)
{
    // BOUCHAUD_PREUVE_HOTE_V1 : quels reveils ecarter.
    //   * preuve de l'hote (KVM) : ceux que l'hote a PROUVE retarder de plus
    //     d'une milliseconde. Un retard inexplique reste compte : le noyau
    //     n'est jamais excuse sans preuve ;
    //   * aucune preuve (TCG) : rien n'est prouve externe ; on ecarte les
    //     reveils retardes sans explication, et un desaccord des deux
    //     jugements rend NON_QUALIFIANT -- jamais OK, jamais un echec impute.
    ph->prouves = ph->inexpliques = ph->ecartes = 0;
    for (int i = 0; i < REVEILS; i++) {
        int prouve = ph->comptes_presents && ph->externe[i] > SEUIL_NS;
        int inexplique = ph->comptes_presents && !prouve && ph->inexplique[i] > SEUIL_NS;
        ph->prouves += prouve;
        ph->inexpliques += inexplique;
        ph->contamine[i] = (char)(prouve || (!ph->preuve_hote && inexplique));
        ph->ecartes += ph->contamine[i];
    }

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
    else if (!ph->comptes_presents)
        snprintf(ph->non_jugeable, sizeof ph->non_jugeable,
                 "%s : comptes du fil absents (/proc/self/fil)", ph->nom);
    else if (ph->sommeils_perdus > 0 || ph->non_apparies > 0 || ph->absences_incompletes)
        snprintf(ph->non_jugeable, sizeof ph->non_jugeable,
                 "%s : comptes du noyau inexploitables (%lld sommeils perdus, ecart de %d sommeils, journal %s)",
                 ph->nom, ph->sommeils_perdus, ph->non_apparies,
                 ph->absences_incompletes ? "incomplet" : "complet");
    else if (ph->prouves > CONTAMINES_MAX)
        snprintf(ph->non_jugeable, sizeof ph->non_jugeable,
                 "%s : %d reveils retardes par l'hote, preuve kvm-steal, sur %d (plus de %d)",
                 ph->nom, ph->prouves, REVEILS, CONTAMINES_MAX);
    else if (ph->ecartes > CONTAMINES_MAX)
        snprintf(ph->non_jugeable, sizeof ph->non_jugeable,
                 "%s : %d reveils retardes sans explication sur %d (plus de %d), aucune preuve de l'hote",
                 ph->nom, ph->ecartes, REVEILS, CONTAMINES_MAX);
}

static void imprime(const struct phase *ph)
{
    printf("  %-24s median %5lld us  p95 %6lld us  p99 %6lld us  pire %6lld us\n",
           ph->nom, ph->tout.median, ph->tout.p95, ph->tout.p99, ph->tout.pire);
    printf("  %-24s median %5lld us  p95 %6lld us  p99 %6lld us  pire %6lld us"
           "  [propres %d/%d, regle=%s]\n",
           ph->nom, ph->propre.median, ph->propre.p95, ph->propre.p99, ph->propre.pire,
           ph->propre.n, REVEILS, ph->preuve_hote ? "preuve-hote" : "sans-preuve");
    // BOUCHAUD_PREUVE_HOTE_V1 : les comptes de la serie, sommes sur ses
    // reveils (us), et le vol de l'hote sur tous les coeurs.
    long long ext = 0, inx = 0, msq = 0, prt = 0, pire_inx = 0;
    for (int i = 0; i < REVEILS; i++) {
        ext += ph->externe[i];
        inx += ph->inexplique[i];
        msq += ph->masque[i];
        prt += ph->pret[i];
        if (ph->inexplique[i] > pire_inx)
            pire_inx = ph->inexplique[i];
    }
    printf("  comptes %s : preuve_hote=%s prouves=%d inexpliques=%d ecartes=%d externe_us=%lld"
           " inexplique_us=%lld inexplique_pire_us=%lld masque_us=%lld pret_us=%lld incompletes=%lld"
           " vol_hote_pour_mille=%lld sommeils=%d sans_sommeil=%d absences=%d\n",
           ph->nom, ph->comptes_presents ? (ph->preuve_hote ? "kvm-steal" : "absente") : "?",
           ph->prouves, ph->inexpliques, ph->ecartes, ext / 1000, inx / 1000, pire_inx / 1000,
           msq / 1000, prt / 1000, ph->incompletes, ph->vol_pour_mille, ph->n_sommeils,
           ph->sans_sommeil, ph->n_absences);
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
    char contamines[128], raison[200], t[200], p[200];
    snprintf(contamines, sizeof contamines, "ecartes repos=%d normal=%d interactif=%d sur %d (%s)",
             repos->ecartes, normale->ecartes, prio->ecartes, REVEILS,
             repos->preuve_hote ? "preuve-hote" : "sans-preuve");

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

// BOUCHAUD_SONDE_RESOLUTION_HORLOGE_V1 -- LE PAS DE L'HORLOGE, PAS CELUI DE
// L'ORDONNANCEUR
//
// La granularite etait le plus petit ecart non nul entre deux lectures
// consecutives, sur les huit premiers ecarts non nuls. Une lecture que
// l'ordonnanceur coupe -- l'appel systeme rend la main a une autre tache
// prete -- donne un ecart de plusieurs millisecondes, et si les huit paires
// non nulles ont ete coupees, la sonde annoncait ce delai comme le pas de
// l'horloge. KVM SMP1 (3adcea3c, A/B reveil precis) : « granularite » de
// 1 030 a 2 531 us dans les DEUX bras, console active ou non, alors que
// `monotonic_ns` lit le TSC a la nanoseconde ; le plancher de jugement (deux
// grains) montait a 4,6 ms et rendait le verdict NON_QUALIFIANT.
//
// Desormais chaque paire est encadree par le TSC, lu en mode utilisateur.
// Une paire est PROPRE si sa duree reelle reste proche de la plus courte
// (deux appels systeme sans rien entre eux) ; seules les paires propres
// disent le pas de l'horloge : le plus petit ecart non nul parmi elles. Les
// paires coupees sont comptees, jamais prises pour le pas. Sans paire propre
// qui avance, ou si le TSC ne se calibre pas sur l'horloge, la resolution est
// NON VERIFIABLE -- la sonde le dit et ne juge pas (NON_QUALIFIANT), elle
// n'invente pas un plancher.

struct paire_horloge {
    long long a_us, b_us;                 // deux lectures consecutives
    unsigned long long tsc_avant, tsc_apres;
};

struct resolution_horloge {
    long long pas_us;      // plus petit ecart non nul d'une paire propre ; -1 : non verifiable
    long long appel_us;    // duree reelle de la paire la plus courte
    int propres;           // paires dont la duree reelle reste proche de `appel_us`
    int coupees;           // paires interrompues (duree reelle bien plus longue)
    int avancees;          // paires propres dont l'horloge a avance
    const char *raison;    // pourquoi le pas n'est pas verifiable
};

// Une paire est coupee au-dela de quatre fois la plus courte, plus 20 us --
// et toujours au-dela de 500 us : si TOUTES les paires sont coupees, la plus
// courte l'est aussi, et un seuil relatif seul les dirait toutes propres.
#define PAIRE_COUPEE_FACTEUR 4
#define PAIRE_COUPEE_MARGE_US 20
#define PAIRE_PROPRE_MAX_US 500

// Pure (testee sur l'hote : test_ordonnanceur_verdict.cpp). `cycles_par_us` :
// le TSC calibre sur l'horloge elle-meme, sur un intervalle long.
static struct resolution_horloge estime_resolution(const struct paire_horloge *p, int n, double cycles_par_us)
{
    struct resolution_horloge r = { -1, -1, 0, 0, 0, "" };
    if (n <= 0) {
        r.raison = "aucune paire";
        return r;
    }
    if (!(cycles_par_us > 0.0)) {
        r.raison = "TSC non calibre sur l'horloge";
        return r;
    }
    long long plus_courte = -1;
    for (int i = 0; i < n; i++) {
        if (p[i].tsc_apres < p[i].tsc_avant || p[i].b_us < p[i].a_us) {
            r.raison = "horloge ou TSC en recul";
            return r;
        }
        long long d = (long long)((double)(p[i].tsc_apres - p[i].tsc_avant) / cycles_par_us);
        if (plus_courte < 0 || d < plus_courte)
            plus_courte = d;
    }
    r.appel_us = plus_courte;
    long long seuil = plus_courte * PAIRE_COUPEE_FACTEUR + PAIRE_COUPEE_MARGE_US;
    if (seuil > PAIRE_PROPRE_MAX_US)
        seuil = PAIRE_PROPRE_MAX_US;
    for (int i = 0; i < n; i++) {
        long long d = (long long)((double)(p[i].tsc_apres - p[i].tsc_avant) / cycles_par_us);
        if (d > seuil) {
            r.coupees++;
            continue;
        }
        r.propres++;
        long long ecart = p[i].b_us - p[i].a_us;
        if (ecart > 0) {
            r.avancees++;
            if (r.pas_us < 0 || ecart < r.pas_us)
                r.pas_us = ecart;
        }
    }
    if (r.propres == 0)
        r.raison = "toutes les paires coupees par l'ordonnanceur";
    else if (r.pas_us < 0)
        r.raison = "aucune paire propre n'a vu l'horloge avancer";
    return r;
}

static inline unsigned long long lit_tsc(void)
{
    unsigned int bas, haut;
    __asm__ volatile("lfence; rdtsc" : "=a"(bas), "=d"(haut) :: "memory");
    return ((unsigned long long)haut << 32) | bas;
}

#define PAIRES_HORLOGE 4000
static struct paire_horloge paires_horloge[PAIRES_HORLOGE];

// Mesure : calibre le TSC sur l'horloge (20 ms), puis jusqu'a 4 000 paires
// encadrees, ou 300 ms. Resultat : `estime_resolution`.
static struct resolution_horloge mesure_resolution(double *cycles_par_us)
{
    long long h0 = maintenant_us();
    unsigned long long t0 = lit_tsc();
    struct timespec dodo = { 0, 20 * 1000 * 1000 };
    nanosleep(&dodo, 0);
    long long h1 = maintenant_us();
    unsigned long long t1 = lit_tsc();
    *cycles_par_us = h1 > h0 && t1 > t0 ? (double)(t1 - t0) / (double)(h1 - h0) : 0.0;
    int n = 0;
    long long fin = maintenant_us() + 300000;
    while (n < PAIRES_HORLOGE) {
        struct paire_horloge *q = &paires_horloge[n++];
        q->tsc_avant = lit_tsc();
        q->a_us = maintenant_us();
        q->b_us = maintenant_us();
        q->tsc_apres = lit_tsc();
        if (q->b_us > fin)
            break;
    }
    return estime_resolution(paires_horloge, n, *cycles_par_us);
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
    // Identite du dormeur, distincte de reveille_par (l'auteur du reveil).
    // Hors des series chronometrees : aucun syscall ajoute entre les reveils.
    printf("ORDONNANCEUR_IDENTITE pid=%ld tid=%ld\n", (long)getpid(), syscall(SYS_gettid));

    // BOUCHAUD_SONDE_RESOLUTION_HORLOGE_V1 : le pas de l'horloge, recoupe par
    // le TSC ; une preemption n'est jamais prise pour lui.
    double cycles_par_us = 0.0;
    struct resolution_horloge h = mesure_resolution(&cycles_par_us);
    if (h.pas_us < 0) {
        printf("  horloge : granularite non verifiable (%s) — appel %lld us, paires propres %d, coupees %d,"
               " %.1f cycles/us\n", h.raison, h.appel_us, h.propres, h.coupees, cycles_par_us);
        printf("VERDICT_SONDE : NON_QUALIFIANT tentatives=0 raison=horloge non verifiable : %s\n", h.raison);
        printf("RESULTAT : NON_QUALIFIANT 0 verification(s) en echec, 0 non qualifiante(s) (0 passees)\n");
        return 2;
    }
    long long grain = h.pas_us;
    printf("  horloge : granularite %lld us — periode visee %d us,"
           " %d processus de calcul, %d tentative(s) au plus ; appel %lld us, paires propres %d"
           " (avancees %d), coupees %d, %.1f cycles/us\n", grain, PERIODE_US, calculs, tentatives,
           h.appel_us, h.propres, h.avancees, h.coupees, cycles_par_us);

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
