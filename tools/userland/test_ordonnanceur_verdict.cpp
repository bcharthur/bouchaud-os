// La qualification des verdicts d'ordonnanceur-probe, sans QEMU
// (BOUCHAUD_SONDE_ORDO_QUALIFICATION_V1).
//
// Les phases sont synthetiques : 300 reveils a 16 ms, des retards choisis, et
// la ligne de ventilation que le noyau ecrirait (liste des reveils contamines,
// echeance arrondie a la milliseconde superieure). Chaque cas negatif est un
// vert que la sonde rendait, ou aurait pu rendre, sans l'avoir mesure.

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

// Une phase propre : retards `retard + (i % 7) * 10` us.
static void remplit(struct phase *ph, const char *nom, long long origine, long long retard)
{
    memset(ph, 0, sizeof *ph);
    ph->nom = nom;
    for (int i = 0; i < REVEILS; i++) {
        ph->attendu[i] = origine + (long long)PERIODE_US * (i + 1);
        ph->retard[i] = retard + (i % 7) * 10;
    }
    ph->attribution_presente = 1;
    snprintf(ph->attribution, sizeof ph->attribution, "reveils=300");
}

// La ligne de ventilation : les reveils `indices` contamines, plus
// `echeances_libres` (echeances hors de tout reveil de la sonde).
static void annonce(struct phase *ph, const int *indices, int n, int tronquee,
                    const long long *echeances_libres, int libres)
{
    char liste[1200] = "";
    size_t l = 0;
    for (int k = 0; k < n + libres; k++) {
        long long echeance = k < n ? ph->attendu[indices[k]] + 700 : echeances_libres[k - n];
        l += (size_t)snprintf(liste + l, sizeof liste - l, "%s%lld:3000", k ? "," : "", echeance);
    }
    snprintf(ph->ventilation, sizeof ph->ventilation,
             "seuil_us=1000 ventiles=%d contamines=%d incompletes=0 livraison_externe=%d:0:0 "
             "liste_tronquee=%d liste=%s",
             n + libres, n + libres, n + libres, tronquee, n + libres ? liste : "-");
}

static void contamine(struct phase *ph, const int *indices, int n, long long retard)
{
    for (int k = 0; k < n; k++)
        ph->retard[indices[k]] = retard;
    annonce(ph, indices, n, 0, NULL, 0);
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
static void sain(void)
{
    remplit(&repos, "au repos", BASE, 600);
    annonce(&repos, NULL, 0, 0, NULL, 0);
    remplit(&normale, "sous charge (normal)", BASE + 10000000, 2000);
    annonce(&normale, NULL, 0, 0, NULL, 0);
    remplit(&prio, "sous charge (interactif)", BASE + 20000000, 900);
    annonce(&prio, NULL, 0, 0, NULL, 0);
}

static void reference_saine(void)
{
    struct bilan b;
    sain();
    VERIFIE(juge_tout(1000, 900, &b) == QUALIFIE_OK, "trois phases propres, la priorite aide : QUALIFIE_OK");
    VERIFIE(b.ok == 4 && b.echecs == 0 && b.non_qualifiantes == 0, "quatre verifications qualifiees");
}

static void temoin_contamine_masque_la_charge(void)
{
    // Run 38569175760 (KVM SMP1) : le temoin est plus lent que la phase sous
    // charge parce que quinze de ses reveils ont attendu un tic perdu. Sur
    // tous les reveils, la charge parait sans effet ; sur les propres, elle
    // degrade la latence. L'ancienne sonde rendait « RESULTAT : 0 ».
    struct bilan b;
    sain();
    int quinze[15];
    for (int k = 0; k < 15; k++)
        quinze[k] = 10 + k * 19;
    contamine(&repos, quinze, 15, 5000);
    normale.retard[0] = 1250;
    enum verdict v = juge_tout(1000, 900, &b);
    VERIFIE(repos.tout.p95 == 5000 && repos.propre.p95 < 700, "le p95 du temoin vient des reveils contamines");
    VERIFIE(v == NON_QUALIFIANT, "temoin contamine qui masque la charge : NON_QUALIFIANT, jamais OK");
    VERIFIE(b.echecs == 0 && b.non_qualifiantes == 2, "les deux comparaisons de priorite ne sont pas jugees");
    VERIFIE(strstr(b.raison, "porte") != NULL, "la raison nomme la porte");
}

static void charge_sans_effet_n_est_pas_une_reussite(void)
{
    // Temoin propre, mais plus lent que la phase sous charge : rien a
    // comparer. Ne rien comparer n'est pas reussir.
    struct bilan b;
    sain();
    remplit(&repos, "au repos", BASE, 1300);
    annonce(&repos, NULL, 0, 0, NULL, 0);
    remplit(&normale, "sous charge (normal)", BASE + 10000000, 700);
    annonce(&normale, NULL, 0, 0, NULL, 0);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "charge sans effet mesurable : NON_QUALIFIANT");
    VERIFIE(strstr(b.raison, "charge sans effet") != NULL, "la raison le dit");
}

static void marge_gonflee_par_le_temoin(void)
{
    // Le bruit admis par la seconde verification est le pire du temoin.
    // Trois reveils contamines du temoin a 9 ms le gonflent : l'aggravation
    // de la priorite (9,5 ms contre 4 ms) passe sur tous les reveils, pas sur
    // les propres.
    struct bilan b;
    sain();
    int trois[3] = {40, 140, 240};
    contamine(&repos, trois, 3, 9000);
    normale.retard[77] = 4000;
    prio.retard[123] = 9500;
    enum verdict v = juge_tout(1000, 900, &b);
    VERIFIE(v == NON_QUALIFIANT, "vert seulement grace au temoin contamine : NON_QUALIFIANT");
    VERIFIE(b.ok == 3 && b.non_qualifiantes == 1, "seule la seconde verification n'est pas qualifiee");
    VERIFIE(strstr(b.raison, "vrai sur tous les reveils, faux sur les reveils propres") != NULL,
            "la raison donne le sens du desaccord");
}

static void echec_qualifie(void)
{
    struct bilan b;
    sain();
    remplit(&prio, "sous charge (interactif)", BASE + 20000000, 3000);
    annonce(&prio, NULL, 0, 0, NULL, 0);
    VERIFIE(juge_tout(1000, 900, &b) == QUALIFIE_ECHEC, "la priorite aggrave tout, sur des reveils propres : QUALIFIE_ECHEC");
    // Le p99 et le pire s'aggravent (premiere), et le pire depasse le pire
    // normal plus le bruit au repos (seconde) : deux echecs qualifies.
    VERIFIE(b.echecs == 2 && b.non_qualifiantes == 0, "deux verifications en echec qualifie");
}

static void echec_du_seul_fait_de_la_contamination(void)
{
    // Trois reveils interactifs retardes par un tic perdu : sur tous les
    // reveils la priorite parait ne rien raccourcir ; sur les propres, elle
    // raccourcit. Ni un echec qualifie, ni une reussite.
    struct bilan b;
    sain();
    int trois[3] = {5, 105, 205};
    contamine(&prio, trois, 3, 20000);
    enum verdict v = juge_tout(1000, 900, &b);
    VERIFIE(v == NON_QUALIFIANT, "echec du seul fait de la contamination : NON_QUALIFIANT");
    VERIFIE(b.echecs == 0, "pas d'echec qualifie impute a l'ordonnanceur");
}

static void phase_trop_contaminee(void)
{
    struct bilan b;
    sain();
    int seize[16];
    for (int k = 0; k < 16; k++)
        seize[k] = k * 18;
    contamine(&repos, seize, 16, 700);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "plus de 15 reveils contamines : phase non jugeable");
    VERIFIE(strstr(repos.non_jugeable, "plus de 15") != NULL, "la phase dit pourquoi");
}

static void contamine_non_apparie(void)
{
    // Une echeance contaminee DANS la serie, entre deux reveils de la sonde :
    // la sonde ne sait pas quel echantillon exclure.
    struct bilan b;
    sain();
    long long entre = normale.attendu[10] + 8000;
    annonce(&normale, NULL, 0, 0, &entre, 1);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "contamine non apparie dans la serie : NON_QUALIFIANT");
    VERIFIE(normale.non_apparies == 1, "compte comme non apparie");
}

static void hors_fenetre_ignore(void)
{
    // Un autre sommeil du fil, avant le premier reveil mesure (dans un appel
    // systeme) : il ne porte sur aucun echantillon.
    struct bilan b;
    sain();
    long long avant = normale.attendu[0] - 3 * PERIODE_US;
    annonce(&normale, NULL, 0, 0, &avant, 1);
    VERIFIE(juge_tout(1000, 900, &b) == QUALIFIE_OK, "sommeil hors de la serie : ignore, QUALIFIE_OK");
    VERIFIE(normale.hors_fenetre == 1 && normale.apparies == 0, "compte hors fenetre");
}

static void attribution_absente(void)
{
    struct bilan b;
    sain();
    prio.attribution_presente = 0;
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "sans attribution du noyau : NON_QUALIFIANT");
}

static void liste_tronquee(void)
{
    struct bilan b;
    sain();
    int deux[2] = {3, 30};
    annonce(&repos, deux, 2, 1, NULL, 0);
    VERIFIE(juge_tout(1000, 900, &b) == NON_QUALIFIANT, "liste tronquee : NON_QUALIFIANT");
}

static void calcul_affame(void)
{
    struct bilan b;
    sain();
    VERIFIE(juge_tout(1000, 0, &b) == QUALIFIE_ECHEC, "calcul affame : QUALIFIE_ECHEC");
    VERIFIE(juge_tout(1000, 99, &b) == QUALIFIE_ECHEC, "moins d'un dixieme du travail : QUALIFIE_ECHEC");
}

int main()
{
    // Les verdicts imprimes par la sonde ne sont pas l'objet du test.
    if (!freopen("/dev/null", "w", stdout))
        return 1;
    reference_saine();
    temoin_contamine_masque_la_charge();
    charge_sans_effet_n_est_pas_une_reussite();
    marge_gonflee_par_le_temoin();
    echec_qualifie();
    echec_du_seul_fait_de_la_contamination();
    phase_trop_contaminee();
    contamine_non_apparie();
    hors_fenetre_ignore();
    attribution_absente();
    liste_tronquee();
    calcul_affame();
    fprintf(stderr, "%d verification(s), %d echec(s)\n", verifications, fautes);
    return fautes ? 1 : 0;
}
