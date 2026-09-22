/*
 * VOIR-CPU : ce qu'un programme utilisateur apprend reellement du materiel.
 *
 * # Pourquoi ce programme existe
 *
 * Le noyau publie maintenant une topologie CPU correcte dans
 * `/sys/devices/system/cpu/online`, `/proc/cpuinfo` et `/proc/stat`. La
 * verification de ces fichiers se fait cote NOYAU -- on lit ce qu'on vient
 * d'ecrire.
 *
 * Ce n'est pas la question qui compte. La question est : que voit un
 * programme en anneau 3, a travers le bac a sable, avec les memes appels que
 * Ladybird et ses bibliotheques ? Un fichier juste mais refuse par la
 * securite donne le meme resultat qu'un fichier faux -- une heuristique qui
 * vaut UN.
 *
 * Ce programme repond par la mesure, et il est ecrit sans bibliotheque : une
 * libc apporterait son propre cache de `sysconf`, et l'on verifierait alors
 * le cache et non le noyau.
 *
 * # Ce qu'il imprime
 *
 *     VOIR_CPU sysfs_online=<...>
 *     VOIR_CPU sysfs_present=<...>
 *     VOIR_CPU cpuinfo_processors=<n>
 *     VOIR_CPU cpuinfo_siblings=<n>
 *     VOIR_CPU cpuinfo_cores=<n>
 *     VOIR_CPU procstat_cpu_lignes=<n>
 *     VOIR_CPU cpuid_logiques=<n>
 *     VOIR_CPU verdict=<...>
 *
 * Chaque ligne REFUSEE se dit `refuse`, et non zero : « le fichier n'est pas
 * lisible » et « il annonce zero processeur » sont deux pannes differentes et
 * ne se reparent pas au meme endroit.
 */

static long appel(long n, long a, long b, long c)
{
    long r;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return r;
}

static long ecris(const char *s, long n) { return appel(1, 1, (long)s, n); }

static long longueur(const char *s)
{
    long n = 0;
    while (s[n]) n++;
    return n;
}

static void dis(const char *s) { ecris(s, longueur(s)); }

static void dis_nombre(long v)
{
    char tampon[24];
    int i = 23;
    tampon[i--] = 0;
    if (v < 0) { dis("-"); v = -v; }
    if (v == 0) tampon[i--] = '0';
    while (v > 0) { tampon[i--] = '0' + (v % 10); v /= 10; }
    dis(&tampon[i + 1]);
}

/* Rend le nombre d'octets lus, ou -1 si le fichier n'est pas lisible. */
static long lis_fichier(const char *chemin, char *tampon, long taille)
{
    long fd = appel(2, (long)chemin, 0 /* O_RDONLY */, 0);
    if (fd < 0) return -1;
    long total = 0;
    for (;;) {
        long n = appel(0, fd, (long)(tampon + total), taille - 1 - total);
        if (n <= 0) break;
        total += n;
        if (total >= taille - 1) break;
    }
    appel(3, fd, 0, 0);
    tampon[total] = 0;
    return total;
}

static int commence_par(const char *s, const char *prefixe)
{
    while (*prefixe) { if (*s++ != *prefixe++) return 0; }
    return 1;
}

/* Le premier entier trouve apres la position donnee, ou -1. */
static long entier_apres(const char *s)
{
    while (*s && (*s < '0' || *s > '9')) s++;
    if (!*s) return -1;
    long v = 0;
    while (*s >= '0' && *s <= '9') { v = v * 10 + (*s - '0'); s++; }
    return v;
}

/* Compte les lignes dont le debut correspond au prefixe. */
static long compte_lignes(const char *texte, const char *prefixe)
{
    long n = 0;
    const char *p = texte;
    int debut_de_ligne = 1;
    while (*p) {
        if (debut_de_ligne && commence_par(p, prefixe)) n++;
        debut_de_ligne = (*p == '\n');
        p++;
    }
    return n;
}

/* La valeur du premier champ `nom` d'un fichier de type cpuinfo. */
static long champ(const char *texte, const char *nom)
{
    const char *p = texte;
    int debut_de_ligne = 1;
    while (*p) {
        if (debut_de_ligne && commence_par(p, nom)) return entier_apres(p + longueur(nom));
        debut_de_ligne = (*p == '\n');
        p++;
    }
    return -1;
}

/*
 * Le nombre de processeurs logiques selon CPUID, sans passer par le noyau.
 *
 * C'est la SEULE mesure de ce programme qui ne depende d'aucun fichier : elle
 * dit ce que le materiel annonce, et sert de temoin. Si elle et `/proc`
 * divergent, c'est le noyau qui se trompe ; si elles s'accordent et que
 * Ladybird voit autre chose, c'est le bac a sable.
 *
 * Feuille 1, EBX[23:16] : le nombre d'identifiants APIC du paquet. Elle est
 * presente partout depuis le Pentium, y compris sous QEMU, ce qui n'est le
 * cas ni de la feuille 0xB ni de la 0x1F.
 */
static long cpuid_logiques(void)
{
    unsigned int eax, ebx, ecx, edx;
    __asm__ volatile("cpuid" : "=a"(eax), "=b"(ebx), "=c"(ecx), "=d"(edx) : "a"(1), "c"(0));
    long n = (ebx >> 16) & 0xff;
    return n > 0 ? n : -1;
}

static void ligne_texte(const char *cle, const char *valeur)
{
    dis("VOIR_CPU ");
    dis(cle);
    dis("=");
    dis(valeur);
    dis("\n");
}

static void ligne_nombre(const char *cle, long valeur)
{
    dis("VOIR_CPU ");
    dis(cle);
    dis("=");
    if (valeur < 0) dis("refuse");
    else dis_nombre(valeur);
    dis("\n");
}

static char tampon[65536];

/* Lit un fichier d'une seule ligne et en retire le saut final. */
static void ligne_fichier(const char *cle, const char *chemin)
{
    if (lis_fichier(chemin, tampon, sizeof tampon) < 0) {
        ligne_texte(cle, "refuse");
        return;
    }
    for (long i = 0; tampon[i]; i++) {
        if (tampon[i] == '\n') { tampon[i] = 0; break; }
    }
    ligne_texte(cle, tampon[0] ? tampon : "vide");
}

void _start(void)
{
    dis("VOIR_CPU debut\n");

    ligne_fichier("sysfs_online", "/sys/devices/system/cpu/online");
    ligne_fichier("sysfs_present", "/sys/devices/system/cpu/present");
    ligne_fichier("sysfs_possible", "/sys/devices/system/cpu/possible");

    long processeurs = -1, siblings = -1, coeurs = -1;
    if (lis_fichier("/proc/cpuinfo", tampon, sizeof tampon) >= 0) {
        processeurs = compte_lignes(tampon, "processor");
        siblings = champ(tampon, "siblings");
        coeurs = champ(tampon, "cpu cores");
    }
    ligne_nombre("cpuinfo_processors", processeurs);
    ligne_nombre("cpuinfo_siblings", siblings);
    ligne_nombre("cpuinfo_cores", coeurs);

    long stat_lignes = -1;
    if (lis_fichier("/proc/stat", tampon, sizeof tampon) >= 0) {
        /* `cpu0`, `cpu1`... et non la ligne `cpu ` agregee. */
        stat_lignes = compte_lignes(tampon, "cpu") - compte_lignes(tampon, "cpu ");
    }
    ligne_nombre("procstat_cpu_lignes", stat_lignes);

    long materiel = cpuid_logiques();
    ligne_nombre("cpuid_logiques", materiel);

    /*
     * LE VERDICT, et c'est la seule ligne qu'un banc a besoin de lire.
     *
     * Un programme utilisateur ne voit le bon nombre de processeurs que si
     * les DEUX sources s'accordent avec le materiel : `/sys` est ce que lit
     * la glibc, `/proc/cpuinfo` ce que lisent les bibliotheques qui comptent
     * les lignes. Une seule des deux suffit a fixer la taille d'un pool.
     */
    if (processeurs < 0) dis("VOIR_CPU verdict=cpuinfo_refuse\n");
    else if (materiel > 0 && processeurs != materiel) dis("VOIR_CPU verdict=desaccord_materiel\n");
    else if (processeurs <= 1) dis("VOIR_CPU verdict=un_seul_processeur\n");
    else dis("VOIR_CPU verdict=coherent\n");

    dis("VOIR_CPU fin\n");
    appel(60, 0, 0, 0);
    for (;;) {}
}
