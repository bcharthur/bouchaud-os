/*
 * TEMPETE-FAUTES : une rafale de fautes de page, sur plusieurs fils.
 *
 * # Pourquoi ce programme existe
 *
 * Le livre des fautes prend son verrou par `try_lock` et jamais `lock` : ce
 * chemin est celui de CHAQUE faute de la machine, et un verrou bloquant y
 * serialiserait seize processeurs sur un compteur de diagnostic -- la mesure
 * creerait la lenteur qu'elle pretend observer.
 *
 * Le prix de ce choix est qu'un echantillon pris pendant qu'un autre coeur
 * ecrit est PERDU. La question que ce programme pose est : combien ?
 *
 * Sous une charge ordinaire la reponse est « presque rien », et cela ne
 * prouve rien : c'est justement pendant une TEMPETE que le verrou est
 * dispute, et c'est la que le livre pourrait devenir inutile.
 *
 * # Ce qu'il fait
 *
 * Il cree plusieurs fils avec `clone`, et chacun touche son propre grand
 * tableau anonyme page par page. Chaque page touchee est une faute `Zero`
 * servie a la demande, et les fils fautent SIMULTANEMENT sur des processeurs
 * differents -- c'est la condition exacte de la contention.
 *
 * Le verdict se lit dans la commande `fautes` du shell :
 *
 *     presentees=<n> non_comptees=<n> perte_pour_mille=<n>
 *
 * Sous un pour cent, le `try_lock` est le bon compromis. Au-dela, il faut un
 * livre par processeur, agrege hors du chemin critique.
 */

/*
 * AUTANT DE FILS QUE DE COEURS, ET PAS UN DE PLUS.
 *
 * Avec huit fils sur quatre coeurs emules, la moitie dort toujours. Une
 * barriere ne peut alors se franchir qu'au rythme des tranches
 * d'ordonnancement : les fils en sortent decales de plusieurs
 * millisecondes, arrivent sur la page l'un apres l'autre, et ne se
 * disputent jamais rien. Le releve ne portait aucune ligne `attente` -- non
 * parce que le noyau ne sait pas faire attendre, mais parce que l'epreuve
 * ne faisait jamais coincider deux fautes.
 *
 * Quatre fils sur quatre coeurs tiennent tous en meme temps sur la machine :
 * ils sortent de la barriere a quelques microsecondes les uns des autres et
 * touchent le meme mot ensemble.
 */
#define FILS 4
#define PAGES_PAR_FIL 2048
#define PAGE 4096
#define PILE_FIL 65536

static long appel6(long n, long a, long b, long c, long d, long e, long f)
{
    long r;
    register long r10 __asm__("r10") = d;
    register long r8 __asm__("r8") = e;
    register long r9 __asm__("r9") = f;
    __asm__ volatile("syscall"
                     : "=a"(r)
                     : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory");
    return r;
}

static long appel(long n, long a, long b, long c) { return appel6(n, a, b, c, 0, 0, 0); }

static long ecris(const char *s, long n) { return appel(1, 1, (long)s, n); }

static void dis(const char *s)
{
    long n = 0;
    while (s[n]) n++;
    ecris(s, n);
}

/* MAP_PRIVATE|MAP_ANONYMOUS : chaque page touchee sera une faute `Zero`. */
static void *reserve(long octets)
{
    long r = appel6(9 /* mmap */, 0, octets, 0x3 /* PROT_READ|WRITE */,
                    0x22 /* MAP_PRIVATE|MAP_ANONYMOUS */, -1, 0);
    return (r < 0 && r > -4096) ? 0 : (void *)r;
}

/*
 * LA ZONE PARTAGEE EST ADOSSEE A UN FICHIER, et ce choix est le seul qui
 * produise reellement la contention.
 *
 * Une page anonyme se peuple en quelques microsecondes : le temps que le
 * deuxieme fil arrive, le premier a fini, et l'enregistrement est deja
 * `Present`. Meme avec une barriere avant CHAQUE page -- ce qui a ete
 * essaye -- le releve ne portait aucune ligne `attente`.
 *
 * Une page de fichier, elle, coute DEUX A TROIS MILLISECONDES sur ce noyau
 * (c'est la mesure de `run_fautes_demande.sh`, et c'est aussi ce qui rend le
 * demarrage de Ladybird si lent). Trois millisecondes sont une eternite :
 * les sept autres fils arrivent tous pendant que le premier charge, trouvent
 * l'enregistrement en `Loading`, et attendent. Le chemin
 * `Loading -> attente -> Present` devient certain au lieu d'improbable.
 *
 * Le fichier projete est le programme LUI-MEME : il est forcement present,
 * sa taille est connue du chargeur, et aucune preparation n'est necessaire
 * cote banc.
 */
static long ouvre_soi(void)
{
    /* openat(AT_FDCWD, "/tempete", O_RDONLY) */
    static const char chemin[] = "/tempete";
    long fd = appel6(257, -100, (long)chemin, 0 /* O_RDONLY */, 0, 0, 0);
    return fd;
}

static void *projette_fichier(long fd, long octets)
{
    long r = appel6(9 /* mmap */, 0, octets, 0x1 /* PROT_READ */,
                    0x02 /* MAP_PRIVATE */, fd, 0);
    return (r < 0 && r > -4096) ? 0 : (void *)r;
}

static volatile long fils_finis = 0;

/*
 * LA ZONE PARTAGEE, et c'est elle qui produit la categorie `Attente`.
 *
 * La premiere phase donne a chaque fil sa propre zone : ils fautent en meme
 * temps, sur des processeurs differents, mais jamais sur la MEME page. Ils
 * se disputent le verrou du livre, pas l'enregistrement de page.
 *
 * La seconde phase les fait tous toucher la meme zone, dans le meme ordre.
 * Un seul charge chaque page ; les autres trouvent l'enregistrement en
 * `Loading` et attendent. C'est le chemin
 * `Loading -> attente -> Present`, celui ou une meme duree etait comptee
 * deux fois -- une fois sous `Attente`, une fois sous `Zero`.
 *
 * QUATRE MONTAGES ONT ETE MESURES AVANT QU'UN SEUL MARCHE.
 *
 * Les fils partagent un espace d'adressage (`CLONE_VM`) : des qu'un fil a
 * peuple une page, les autres n'y fautent PLUS DU TOUT. Pour qu'une faute en
 * attende une autre, deux fils doivent donc toucher la meme page PENDANT sa
 * fenetre de chargement.
 *
 *   zone anonyme, sans barriere   `attente` n=10..13, mais un releve sur
 *                                 trois n'en portait aucune
 *   rendez-vous par vague         aucune attente
 *   rendez-vous par page          aucune attente, et la mesure dit pourquoi :
 *                                 `arrives=256 expires=128`, la moitie des
 *                                 rendez-vous manques
 *   fichier + UN rendez-vous      `attente` revient
 *
 * Ce qui casse les trois premiers est toujours la meme chose : un rendez-vous
 * qui EXPIRE laisse partir son fil en avance, `arrives` grimpe, les suivants
 * se debloquent trop tot, et la desynchronisation se propage. Multiplier les
 * rendez-vous multipliait donc les occasions de tout desynchroniser.
 *
 * Le montage retenu n'en pose qu'UN, avant le balayage, et confie la
 * resynchronisation au chargement lui-meme : chaque page de fichier coute des
 * centaines de microsecondes, si bien que les retardataires trouvent
 * l'enregistrement en `Loading` et attendent -- ce qui est precisement le
 * chemin qu'on veut voir.
 *
 * La contention reste probabiliste sous QEMU. Le banc ne la revendique donc
 * pas : quand aucune attente n'a lieu, il le dit.
 */
#define PAGES_PARTAGEES 64
static const char *zone_partagee = 0;

static volatile long arrives = 0;
static volatile long expires = 0;

/*
 * LE PLAFOND D'ATTENTE EST CALIBRE POUR L'EMULATION, PAS POUR LE SILICIUM.
 *
 * La premiere version tournait deux cents millions de fois, et il y avait
 * alors une attente par page. Sur le Trigkey c'est une fraction de seconde ;
 * sous QEMU en traduction logicielle, avec huit fils sur quatre coeurs
 * emules, c'etait plusieurs dizaines de secondes CHACUNE. Le banc expirait,
 * le releve ne montrait plus aucune faute apres les premieres, et le tableau
 * de bord du noyau accusait `clone` de tenir un verrou : tout ce qu'on voyait
 * etait le symptome de fils qui tournaient dans le vide en anneau 3.
 *
 * UNE MESURE MAL CALIBREE ACCUSE LE CODE QU'ELLE OBSERVE. Il n'y a plus
 * qu'un seul rendez-vous, et il peut donc etre genereux.
 *
 * Une attente qui expire n'est pas une erreur : le fil passe a la page
 * suivante. Une page ratee coute une occasion de contention, pas le banc.
 */
#define TOURS_MAX 40000000

/*
 * L'effectif est une CONSTANTE, et c'est ce qui evite de le publier depuis le
 * fil principal -- publication qui n'arrivait jamais tant que le principal
 * etait dans sa boucle de `clone`, et qui bloquait alors tous les autres.
 *
 * Barriere MONOTONE : la cible du n-ieme rendez-vous est n fois l'effectif.
 * Pas de remise a zero, donc pas de course entre le dernier arrivant d'un
 * rendez-vous et le premier du suivant.
 */
static void rendez_vous(long numero)
{
    __atomic_fetch_add(&arrives, 1, __ATOMIC_SEQ_CST);
    long cible = (long)FILS * numero;
    for (long t = 0; t < TOURS_MAX; t++) {
        if (__atomic_load_n(&arrives, __ATOMIC_SEQ_CST) >= cible) return;
        __asm__ volatile("pause");
    }
    __atomic_fetch_add(&expires, 1, __ATOMIC_SEQ_CST);
}

/*
 * Le corps d'un fil : toucher chaque page de sa zone.
 *
 * Une page par 4096 octets, et l'ecriture est VOLATILE : sans cela le
 * compilateur supprimerait la boucle entiere, et le programme ne produirait
 * aucune faute.
 */
static void touche_partage(void)
{
    if (!zone_partagee) return;
    /*
     * UN SEUL rendez-vous, et il est place AVANT le balayage.
     *
     * Une barriere par page a ete essayee : elle ne tient pas. Un rendez-vous
     * qui expire laisse partir son fil en avance, `arrives` grimpe, les
     * suivants se debloquent trop tot, et la desynchronisation se propage --
     * la mesure a donne `arrives=256 expires=128`, soit la moitie des
     * rendez-vous manques, et pas une seule faute en attente.
     *
     * Les fils balaient les memes pages dans le meme ordre, a la meme
     * vitesse, et chaque page de fichier coute des centaines de microsecondes
     * a charger. Une fois partis ensemble, ils restent ensemble : c'est le
     * chargement lui-meme qui les resynchronise a chaque page, puisque les
     * retardataires trouvent l'enregistrement en `Loading` et attendent --
     * ce qui est precisement le chemin qu'on veut voir.
     */
    rendez_vous(1);
    for (long i = 0; i < PAGES_PARTAGEES; i++) {
        /* PROT_READ : on LIT. Le `volatile` empeche la suppression. */
        volatile const char *p = zone_partagee + i * PAGE;
        (void)*p;
    }
}

static void travaille(char *zone)
{
    for (long i = 0; i < PAGES_PAR_FIL; i++) {
        volatile char *p = zone + i * PAGE;
        *p = (char)(i & 0x7f);
    }
    touche_partage();
    __atomic_fetch_add(&fils_finis, 1, __ATOMIC_SEQ_CST);
    appel(60 /* exit */, 0, 0, 0);
    for (;;) {}
}

void _start(void)
{
    dis("TEMPETE debut\n");

    long fd = ouvre_soi();
    if (fd >= 0)
        zone_partagee = (const char *)projette_fichier(fd, PAGES_PARTAGEES * PAGE);
    if (!zone_partagee)
        dis("TEMPETE ATTENTION zone partagee absente\n");

    char *zones[FILS];
    for (int i = 0; i < FILS; i++) {
        zones[i] = (char *)reserve(PAGES_PAR_FIL * PAGE);
        if (!zones[i]) {
            dis("TEMPETE FAIL mmap\n");
            appel(60, 1, 0, 0);
        }
    }

    /*
     * CLONE_VM : les fils PARTAGENT l'espace d'adressage, et c'est ce qui
     * fait la contention. Des processus separes fauteraient chacun dans le
     * sien, sans jamais se disputer le meme enregistrement de page.
     */
    const long CLONE_VM = 0x100, CLONE_FS = 0x200, CLONE_FILES = 0x400;
    const long CLONE_SIGHAND = 0x800, CLONE_THREAD = 0x10000;
    long drapeaux = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD;

    int lances = 0;
    for (int i = 0; i < FILS; i++) {
        char *pile = (char *)reserve(PILE_FIL);
        if (!pile) break;
        /* La pile croit vers le bas, et l'ABI exige seize octets d'alignement. */
        void **sommet = (void **)(pile + PILE_FIL - 16);
        sommet[0] = (void *)zones[i];
        long r = appel6(56 /* clone */, drapeaux, (long)sommet, 0, 0, 0, 0);
        if (r == 0) {
            /* Le fils : son argument est au sommet de sa pile. */
            char *mienne;
            __asm__ volatile("mov (%%rsp), %0" : "=r"(mienne));
            travaille(mienne);
        }
        if (r > 0) lances++;
    }

    /*
     * Un fil qui manque fausse chaque barriere : la cible `FILS * numero` ne
     * serait jamais atteinte et tous les rendez-vous expireraient. Le dire
     * tout de suite vaut mieux qu'un releve sans contention qu'on
     * interpreterait comme une absence de defaut.
     */
    if (lances != FILS)
        dis("TEMPETE ATTENTION fils manquants\n");

    /*
     * LE FIL PRINCIPAL NE PARTICIPE PAS A LA TEMPETE, et c'est deliberé.
     *
     * Le noyau EPINGLE le fil racine d'un lancement synchrone sur le coeur
     * appelant (`run()` dans lifecycle.rs : il doit revenir sur la pile noyau
     * de ce coeur). Ses pthreads, eux, naissent avec l'affinite machine
     * complete. Le principal partage donc son coeur avec le bureau et le
     * shell, et prend du retard sur les autres.
     *
     * Quand il participait aux barrieres, la MOITIE d'entre elles expirait :
     * les trois fils libres l'attendaient plusieurs dizaines de millisecondes,
     * renoncaient, et touchaient la page chacun a son tour. Aucune faute n'en
     * rencontrait une autre. Le mesure a ete faite : `arrives=256 expires=128`
     * sur deux cent cinquante-six rendez-vous.
     *
     * Il se contente maintenant d'attendre la fin des fils.
     */
    for (long tours = 0; tours < 200000000; tours++) {
        if (__atomic_load_n(&fils_finis, __ATOMIC_SEQ_CST) >= lances) break;
        __asm__ volatile("pause");
    }

    {
        char tampon[96];
        long n = 0;
        const char *tete = "TEMPETE bilan arrives=";
        while (tete[n]) { tampon[n] = tete[n]; n++; }
        long v = __atomic_load_n(&arrives, __ATOMIC_SEQ_CST);
        char chiffres[24]; long c = 0;
        if (v == 0) chiffres[c++] = '0';
        while (v > 0) { chiffres[c++] = (char)('0' + v % 10); v /= 10; }
        while (c > 0) tampon[n++] = chiffres[--c];
        const char *milieu = " expires=";
        long m = 0; while (milieu[m]) tampon[n++] = milieu[m++];
        v = __atomic_load_n(&expires, __ATOMIC_SEQ_CST);
        c = 0;
        if (v == 0) chiffres[c++] = '0';
        while (v > 0) { chiffres[c++] = (char)('0' + v % 10); v /= 10; }
        while (c > 0) tampon[n++] = chiffres[--c];
        tampon[n++] = '\n';
        ecris(tampon, n);
    }
    dis("TEMPETE fin\n");
    appel(60, 0, 0, 0);
    for (;;) {}
}
