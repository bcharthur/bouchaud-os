// La qualification des verdicts d'ordonnanceur-probe, sans QEMU
// (BOUCHAUD_SONDE_ORDO_QUALIFICATION_V1, BOUCHAUD_PREUVE_HOTE_V1).
//
// Les phases sont synthetiques : 300 reveils a 16 ms, des retards choisis, et
// pour chaque reveil les parts que les comptes du fil auraient rendues -- ou
// les sommeils que le noyau aurait ranges (`repartit`). Chaque cas negatif est un vert que la sonde rendait, ou
// aurait pu rendre, sans l'avoir mesure -- ou un rouge du noyau qu'elle aurait
// excuse sans preuve.

#define main sonde_main
#include "ordonnanceur-probe.c"
#undef main

static int fautes = 0;
static int verifications = 0;

#define VERIFIE(condition, quoi)                                                 \
    do {                                                                         \
        verifications++;                                                         \
        if (!(condition)) {                                                      \
            fautes++;                                                            \
            fprintf(stderr, "ECHEC %s:%d %s\n", __FILE__, __LINE__, quoi);       \
        }                                                                        \
    } while (0)

static const long long BASE = 50000000;

// Une phase sans retard explique ni inexplique : retards `retard + (i % 7) *
// 10` us. `preuve` : l'hote publie le vol (KVM) ou non (TCG).
static void remplit(struct phase *ph, const char *nom, long long origine, long long retard, int preuve)
{
    memset(ph, 0, sizeof *ph);
    ph->nom = nom;
    for (int i = 0; i < REVEILS; i++) {
        ph->attendu[i] = origine + (long long)PERIODE_US * (i + 1);
        ph->retard[i] = retard + (i % 7) * 10;
    }
    ph->attribution_presente = 1;
    ph->comptes_presents = 1;
    ph->preuve_hote = preuve;
    snprintf(ph->attribution, sizeof ph->attribution, "reveils=300");
    snprintf(ph->ventilation, sizeof ph->ventilation, "seuil_us=1000");
}

// Des reveils que l'hote a PROUVE retarder (vol KVM de 3 ms dans leur
// fenetre).
static void prouve(struct phase *ph, const int *indices, int n, long long retard)
{
    for (int k = 0; k < n; k++) {
        ph->retard[indices[k]] = retard;
        ph->externe[indices[k]] = 3000000;
    }
}

// Des reveils retardes par des absences de battement que ni le noyau ni
// l'hote n'expliquent.
static void inexplique(struct phase *ph, const int *indices, int n, long long retard)
{
    for (int k = 0; k < n; k++) {
        ph->retard[indices[k]] = retard;
        ph->inexplique[indices[k]] = 3000000;
    }
}

static struct phase repos, normale, prio;

static enum verdict juge_tout(unsigned long long tours_normale, unsigned long long tours_prio,
                              struct bilan *b)
{
    finalise_phase(&repos);
    finalise_phase(&normale);
    finalise_phase(&prio);
    return evalue(&repos, &normale, &prio, tours_normale, tours_prio, 1, b);
}

// Trois phases propres ou la priorite aide : la reference.
static void sain(int preuve)
{
    remplit(&repos, "au repos", BASE, 600, preuve);
    remplit(&normale, "sous charge (normal)", BASE + 10000000, 2000, preuve);
    remplit(&prio, "sous charge (interactif)", BASE + 20000000, 900, preuve);
}

static void reference_saine(void)
{
    struct bilan b;
    sain(1);
    VERIFIE(juge_tout(1000, 900, &b) == QUALIFIE_OK, "KVM : trois phases propres, la priorite aide : QUALIFIE_OK");
    VERIFIE(b.ok == 4 && b.echecs == 0 && b.non_qualifiantes == 0, "quatre verifications qualifiees");
    sain(0);
    VERIFIE(juge_tout(1000, 900, &b) == QUALIFIE_OK, "TCG : sans aucun retard inexplique, la mesure qualifie aussi");
}

static void temoin_prouve_masque_la_charge(void)
{
    // Run 38569175760 (KVM SMP1) : le temoin est plus lent que la phase sous
    // charge parce que quinze de ses reveils ont attendu l'hote. Sur tous les
    // reveils, la charge parait sans effet ; sur les propres, elle degrade
    // la latence. L'ancienne sonde rendait « RESULTAT : 0 ».
    struct bilan b;
    sain(1);
    int quinze[15];
    for (int k = 0; k < 15; k++)
        quinze[k] = 10 + k * 19;
    prouve(&repos, quinze, 15, 5000);
    normale.retard[0] = 1250;
    enum verdict v = juge_tout(1000, 900, &b);
    VERIFIE(repos.tout.p95 == 5000 && repos.propre.p95 < 700, "le p95 du temoin vient des reveils prouves");
    VERIFIE(v == NON_QUALIFIANT, "temoin retarde par l'hote qui masque la charge : NON_QUALIFIANT, jamais OK");
    VERIFIE(b.echecs == 0 && b.non_qualifiantes == 2, "les deux comparaisons de priorite ne sont pas jugees");
    VERIFIE(strstr(b.raison, "porte") != NULL, "la raison nomme la porte");
}

static void temoin_inexplique_sous_kvm_n_est_pas_excuse(void)
{
    // Les memes quinze reveils lents, mais l'hote n'a RIEN vole : la lenteur
    // n'est pas ecartee. Le temoin reste lent, la charge ne se compare pas --
    // et la sonde ne pretend pas que le temoin etait propre.
    struct bilan b;
    sain(1);
    int quinze[15];
    for (int k = 0; k < 15; k++)
        quinze[k] = 10 + k * 19;
    inexplique(&repos, quinze, 15, 5000);
    juge_tout(1000, 900, &b);
    VERIFIE(repos.propre.p95 == 5000 && repos.ecartes == 0, "KVM : aucun reveil inexplique n'est ecarte");
    VERIFIE(repos.inexpliques == 15, "les quinze sont comptes inexpliques");
    VERIFIE(repos.non_jugeable[0] == '\0', "KVM : des inexpliques ne rendent pas la phase non jugeable");
}

static void charge_sans_effet_n_est_pas_une_reussite(void)
{
    struct bilan b;
    sain(1);
    remplit(&repos, "au repos", BASE, 1300, 1);
    remplit(&normale, "sous charge (normal)", BASE + 10000000, 700, 1);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "charge sans effet mesurable : NON_QUALIFIANT");
    VERIFIE(strstr(b.raison, "charge sans effet") != NULL, "la raison le dit");
}

static void marge_gonflee_par_le_temoin(void)
{
    // Le bruit admis par la seconde verification est le pire du temoin.
    // Trois reveils du temoin retardes par l'hote a 9 ms le gonflent.
    struct bilan b;
    sain(1);
    int trois[3] = {40, 140, 240};
    prouve(&repos, trois, 3, 9000);
    normale.retard[77] = 4000;
    prio.retard[123] = 9500;
    enum verdict v = juge_tout(1000, 900, &b);
    VERIFIE(v == NON_QUALIFIANT, "vert seulement grace au temoin retarde par l'hote : NON_QUALIFIANT");
    VERIFIE(b.ok == 3 && b.non_qualifiantes == 1, "seule la seconde verification n'est pas qualifiee");
    VERIFIE(strstr(b.raison, "vrai sur tous les reveils, faux sur les reveils propres") != NULL,
            "la raison donne le sens du desaccord");
}

static void echec_qualifie(void)
{
    struct bilan b;
    sain(1);
    remplit(&prio, "sous charge (interactif)", BASE + 20000000, 3000, 1);
    VERIFIE(juge_tout(1000, 900, &b) == QUALIFIE_ECHEC, "la priorite aggrave tout, sur des reveils propres : QUALIFIE_ECHEC");
    VERIFIE(b.echecs == 2 && b.non_qualifiantes == 0, "deux verifications en echec qualifie");
}

static void echec_du_seul_fait_de_l_hote(void)
{
    // Trois reveils interactifs retardes par l'hote, preuve a l'appui : sur
    // tous les reveils la priorite parait ne rien raccourcir ; sur les
    // propres, elle raccourcit. Ni un echec qualifie, ni une reussite.
    struct bilan b;
    sain(1);
    int trois[3] = {5, 105, 205};
    prouve(&prio, trois, 3, 20000);
    enum verdict v = juge_tout(1000, 900, &b);
    VERIFIE(v == NON_QUALIFIANT, "echec du seul fait de l'hote prouve : NON_QUALIFIANT");
    VERIFIE(b.echecs == 0, "pas d'echec qualifie impute a l'ordonnanceur");
}

static void echec_inexplique_sous_kvm_est_qualifie(void)
{
    // BOUCHAUD_PREUVE_HOTE_V1 -- le cas que la revue exige : les MEMES trois
    // reveils retardes, mais sans aucun vol de l'hote. Rien ne permet de les
    // mettre au compte de QEMU : l'echec reste au noyau.
    struct bilan b;
    sain(1);
    int trois[3] = {5, 105, 205};
    inexplique(&prio, trois, 3, 20000);
    enum verdict v = juge_tout(1000, 900, &b);
    VERIFIE(v == QUALIFIE_ECHEC, "KVM, retard inexplique : QUALIFIE_ECHEC, jamais excuse");
    VERIFIE(b.echecs >= 1, "l'echec est impute");
    VERIFIE(prio.ecartes == 0 && prio.inexpliques == 3, "aucun reveil ecarte, trois inexpliques");
}

static void echec_inexplique_sans_preuve_ne_qualifie_rien(void)
{
    // Sous TCG, aucune preuve de l'hote n'existe : les memes trois reveils
    // inexpliques peuvent etre le noyau ou l'emulateur. Ni OK ni echec.
    struct bilan b;
    sain(0);
    int trois[3] = {5, 105, 205};
    inexplique(&prio, trois, 3, 20000);
    enum verdict v = juge_tout(1000, 900, &b);
    VERIFIE(v == NON_QUALIFIANT, "TCG, retard inexplique decisif : NON_QUALIFIANT");
    VERIFIE(b.echecs == 0, "TCG : aucun echec impute sans preuve");
}

static void un_vol_sous_le_seuil_n_ecarte_rien(void)
{
    struct bilan b;
    sain(1);
    prio.externe[50] = SEUIL_NS;
    prio.retard[50] = 20000;
    juge_tout(1000, 900, &b);
    VERIFIE(prio.ecartes == 0 && prio.prouves == 0, "une milliseconde de vol, pas plus : rien d'ecarte");
}

static void phase_trop_retardee_par_l_hote(void)
{
    struct bilan b;
    sain(1);
    int seize[16];
    for (int k = 0; k < 16; k++)
        seize[k] = k * 18;
    prouve(&repos, seize, 16, 700);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "plus de 15 reveils prouves : phase non jugeable");
    VERIFIE(strstr(repos.non_jugeable, "preuve kvm-steal") != NULL, "la phase dit pourquoi, preuve nommee");
}

static void seize_inexpliques(void)
{
    struct bilan b;
    int seize[16];
    for (int k = 0; k < 16; k++)
        seize[k] = k * 18;
    // TCG : la phase ne peut pas etre jugee, et la raison ne blame pas l'hote.
    sain(0);
    inexplique(&repos, seize, 16, 700);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "TCG, 16 inexpliques : non jugeable");
    VERIFIE(strstr(repos.non_jugeable, "sans explication") != NULL
                && strstr(repos.non_jugeable, "aucune preuve de l'hote") != NULL,
            "TCG : la raison dit « sans explication », pas « hote »");
    // KVM : comptes, jamais ecartes, la phase est jugee.
    sain(1);
    inexplique(&repos, seize, 16, 700);
    juge_tout(1000, 900, &b);
    VERIFIE(repos.non_jugeable[0] == '\0' && repos.ecartes == 0, "KVM, 16 inexpliques : jugee, rien d'ecarte");
}

static void attribution_ou_comptes_absents(void)
{
    struct bilan b;
    sain(1);
    prio.attribution_presente = 0;
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "sans attribution du noyau : NON_QUALIFIANT");
    sain(1);
    normale.comptes_presents = 0;
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "sans comptes du fil : NON_QUALIFIANT");
    VERIFIE(strstr(normale.non_jugeable, "/proc/self/fil") != NULL, "la raison nomme le fichier");
}

// ---------------------------------------------------------------------------
// La repartition des comptes ranges par le noyau a l'entree de chaque sommeil.

// Une phase dont chaque reveil a dormi et a son sommeil : entree 15 ms avant
// l'instant attendu, cumuls nuls, aucune absence ; fin de serie 1 ms apres
// le dernier reveil.
static void sommeils_reguliers(struct phase *ph)
{
    ph->n_sommeils = REVEILS;
    ph->sommeils_perdus = 0;
    ph->n_absences = 0;
    ph->absences_incompletes = 0;
    memset(&ph->final, 0, sizeof ph->final);
    ph->final.a_ns = (ph->attendu[REVEILS - 1] + 1000) * 1000;
    for (int k = 0; k < REVEILS; k++) {
        memset(&ph->sommeils[k], 0, sizeof ph->sommeils[k]);
        ph->sommeils[k].entree_us = ph->attendu[k] - 15000;
        ph->sommeils[k].echeance_us = ph->attendu[k] + 700;
        ph->dormi[k] = 1;
    }
}

static void absence(struct phase *ph, long long fin_us, char cause, long long part_ns)
{
    ph->absence_fin_us[ph->n_absences] = fin_us;
    ph->absence_cause[ph->n_absences] = cause;
    ph->absence_part_ns[ph->n_absences++] = part_ns;
}

static void repartition(void)
{
    struct bilan b;
    sain(1);
    sommeils_reguliers(&prio);
    // Entre l'entree du sommeil 41 et celle du 42, la livraison prouvee par
    // l'hote avance de 3 ms ; entre 100 et 101, 4 ms inexpliquees.
    for (int k = 42; k < REVEILS; k++)
        prio.sommeils[k].externe_ns = 3000000;
    for (int k = 101; k < REVEILS; k++)
        prio.sommeils[k].inexplique_ns = 4000000;
    prio.final.externe_ns = 3000000;
    prio.final.inexplique_ns = 4000000;
    // Une absence close pendant que la sonde tenait son coeur, dans
    // l'intervalle du reveil 200 (apres son entree, avant celle du 201),
    // prouvee ; une autre, masquee, dans celui du 250 ; une inexpliquee
    // apres le dernier sommeil, avant la fin de serie.
    absence(&prio, prio.attendu[200] + 300, 'E', 2200000);
    absence(&prio, prio.attendu[250] + 100, 'M', 5000000);
    absence(&prio, prio.attendu[REVEILS - 1] + 500, 'I', 1500000);
    repartit(&prio);
    VERIFIE(prio.externe[41] == 3000000 && prio.externe[40] == 0 && prio.externe[42] == 0,
            "la part prouvee tombe dans l'intervalle du reveil 41, seul");
    VERIFIE(prio.inexplique[100] == 4000000 && prio.inexplique[99] == 0, "l'inexplique dans celui du reveil 100");
    VERIFIE(prio.externe[200] == 2200000 && prio.externe[199] == 0 && prio.externe[201] == 0,
            "l'absence du journal tombe dans l'intervalle qui contient sa fin");
    VERIFIE(prio.masque[250] == 5000000, "une absence masquee est rangee masquee");
    VERIFIE(prio.inexplique[REVEILS - 1] == 1500000, "le dernier intervalle se ferme sur la fin de serie");
    VERIFIE(prio.non_apparies == 0 && prio.sans_sommeil == 0, "tout apparie");
    juge_tout(1000, 900, &b);
    VERIFIE(prio.prouves == 2 && prio.inexpliques == 2, "deux prouves, deux inexpliques");

    // Un reveil qui n'a pas dormi tombe dans l'intervalle du precedent.
    sain(1);
    sommeils_reguliers(&prio);
    for (int k = 60; k < REVEILS - 1; k++)
        prio.sommeils[k] = prio.sommeils[k + 1];
    prio.n_sommeils = REVEILS - 1;
    prio.dormi[60] = 0;
    for (int k = 60; k < prio.n_sommeils; k++)
        prio.sommeils[k].externe_ns = 2500000;
    prio.final.externe_ns = 2500000;
    repartit(&prio);
    VERIFIE(prio.sans_sommeil == 1, "un reveil sans sommeil");
    VERIFIE(prio.externe[59] == 2500000 && prio.externe[60] == 2500000 && prio.externe[61] == 0,
            "le reveil sans sommeil partage l'intervalle qui le contient");

    // L'appariement se fait dans l'ordre : une echeance du noyau decalee
    // (tics perdus, autre horloge) ne change rien.
    sain(1);
    sommeils_reguliers(&prio);
    for (int k = 0; k < REVEILS; k++)
        prio.sommeils[k].echeance_us -= 40000;
    repartit(&prio);
    VERIFIE(prio.non_apparies == 0, "echeances decalees : appariement par l'ordre, intact");

    // Un sommeil de trop (ou de moins) : on ne sait plus lequel revient a
    // qui -- la phase ne peut pas etre jugee.
    sain(1);
    sommeils_reguliers(&prio);
    prio.dormi[10] = 0;
    repartit(&prio);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT && prio.non_apparies == 1,
            "un sommeil de plus que de reveils dormis : NON_QUALIFIANT");
    sain(1);
    sommeils_reguliers(&prio);
    prio.sommeils_perdus = 3;
    repartit(&prio);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "sommeils perdus par l'enregistreur : NON_QUALIFIANT");
    sain(1);
    sommeils_reguliers(&prio);
    prio.absences_incompletes = 1;
    repartit(&prio);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "journal incomplet : NON_QUALIFIANT");
}

static void calcul_affame(void)
{
    struct bilan b;
    sain(1);
    VERIFIE(juge_tout(1000, 0, &b) == QUALIFIE_ECHEC, "calcul affame : QUALIFIE_ECHEC");
    VERIFIE(juge_tout(1000, 99, &b) == QUALIFIE_ECHEC, "moins d'un dixieme du travail : QUALIFIE_ECHEC");
}

int main()
{
    // Les verdicts imprimes par la sonde ne sont pas l'objet du test.
    if (!freopen("/dev/null", "w", stdout))
        return 1;
    reference_saine();
    temoin_prouve_masque_la_charge();
    temoin_inexplique_sous_kvm_n_est_pas_excuse();
    charge_sans_effet_n_est_pas_une_reussite();
    marge_gonflee_par_le_temoin();
    echec_qualifie();
    echec_du_seul_fait_de_l_hote();
    echec_inexplique_sous_kvm_est_qualifie();
    echec_inexplique_sans_preuve_ne_qualifie_rien();
    un_vol_sous_le_seuil_n_ecarte_rien();
    phase_trop_retardee_par_l_hote();
    seize_inexpliques();
    attribution_ou_comptes_absents();
    repartition();
    calcul_affame();
    fprintf(stderr, "%d verification(s), %d echec(s)\n", verifications, fautes);
    return fautes ? 1 : 0;
}
