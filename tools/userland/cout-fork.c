/*
 * COUT-FORK : ce que coute un `fork` quand le pere est gros.
 *
 * # La question
 *
 * `AddressSpace::duplicate` recopie IMMEDIATEMENT chaque page possedee par le
 * pere -- une allocation de frame et un `copy_nonoverlapping` de quatre
 * kibioctets par page. Le commentaire du noyau l'assume : « le cas d'usage
 * courant (`fork` suivi d'un `execve`) paie ici une copie que le COW aurait
 * economisee ; c'est un compromis assume, pas un oubli. »
 *
 * Assume, oui. Mais jamais CHIFFRE. Or c'est exactement le chemin que prend
 * chaque processus du navigateur : `Core::Process::spawn` fait `fork` puis
 * `execve`, et le premier WebWorker met vingt-six secondes entre la demande de
 * lancement et le retour du lancement.
 *
 * Si le cout croit avec la taille residente du pere, alors un navigateur --
 * qui est le plus gros processus de la machine -- paie le plus cher, et il le
 * paie a CHAQUE lancement de service.
 *
 * # La mesure
 *
 * Pour quatre tailles residentes, trois `fork` dont l'enfant ne fait QUE
 * mourir. Le pere chronometre l'appel lui-meme, pas la vie de l'enfant :
 *
 *     COUT_FORK rss_mio=4   essai=1 fork_us=...
 *     COUT_FORK rss_mio=256 essai=1 fork_us=...
 *
 * Une droite passant par l'origine signerait la recopie : le cout par
 * mebioctet est alors constant, et il suffit de le multiplier par la taille du
 * navigateur pour retrouver les vingt-six secondes.
 *
 * Sans libc : ce qu'on mesure est le noyau, et une libc apporterait ses
 * propres allocations au demarrage.
 */

static long appel6(long n,long a,long b,long c,long d,long e,long f){long r;register long r10 __asm__("r10")=d;register long r8 __asm__("r8")=e;register long r9 __asm__("r9")=f;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c),"r"(r10),"r"(r8),"r"(r9):"rcx","r11","memory");return r;}
static long appel(long n,long a,long b,long c){return appel6(n,a,b,c,0,0,0);}
static void dis(const char*s){long n=0;while(s[n])n++;appel(1,1,(long)s,n);}

struct ts { long sec; long nsec; };

static long maintenant_us(void)
{
    struct ts t = {0,0};
    appel(228 /* clock_gettime */, 1 /* CLOCK_MONOTONIC */, (long)&t, 0);
    return t.sec * 1000000L + t.nsec / 1000L;
}

/* Ecrit un entier decimal sans libc. */
static char *entier(char *p, long v)
{
    if (v < 0) { *p++ = '-'; v = -v; }
    char tampon[24];
    int n = 0;
    do { tampon[n++] = (char)('0' + (v % 10)); v /= 10; } while (v);
    while (n) *p++ = tampon[--n];
    return p;
}

#define PAGE 4096L

/* Les tailles sont croissantes : chaque etape garde la precedente projetee,
   si bien que la taille residente du pere augmente reellement d'un palier a
   l'autre. Les liberer entre deux mesures rendrait la courbe plate. */
static const long TAILLES_MIO[] = { 4, 16, 64, 256 };
#define PALIERS 4
#define ESSAIS 3

void _start(void)
{
    dis("=== COUT FORK DEBUT ===\n");

    long cumul_mio = 0;
    for (int palier = 0; palier < PALIERS; palier++) {
        long ajout_mio = TAILLES_MIO[palier] - cumul_mio;
        long octets = ajout_mio * 1024L * 1024L;

        /* MAP_PRIVATE|MAP_ANONYMOUS, lecture/ecriture. */
        long base = appel6(9 /* mmap */, 0, octets, 3 /* PROT_READ|WRITE */,
                           0x22 /* MAP_PRIVATE|MAP_ANONYMOUS */, -1, 0);
        if ((unsigned long)base >= (unsigned long)-4095L) {
            char refus[64];
            char *q = refus;
            const char *r = "COUT_FORK mmap refuse code=";
            while (*r) *q++ = *r++;
            q = entier(q, base);
            *q++ = '\n';
            appel(1, 1, (long)refus, q - refus);
            appel(60, 1, 0, 0);
        }
        /* Toucher chaque page : sans cela rien n'est resident, et `duplicate`
           n'aurait rien a copier -- la mesure serait vide de sens. */
        volatile char *zone = (volatile char *)base;
        for (long o = 0; o < octets; o += PAGE)
            zone[o] = (char)(o >> 12);
        cumul_mio = TAILLES_MIO[palier];

        for (int essai = 1; essai <= ESSAIS; essai++) {
            long t0 = maintenant_us();
            long pid = appel(57 /* fork */, 0, 0, 0);
            if (pid == 0) {
                /* L'enfant ne fait RIEN : tout ce qui est mesure est la
                   duplication elle-meme, pas le travail de l'enfant. */
                appel(60 /* exit */, 0, 0, 0);
                for (;;) {}
            }
            long t1 = maintenant_us();
            if (pid > 0)
                appel6(61 /* wait4 */, pid, 0, 0, 0, 0, 0);

            char ligne[128];
            char *p = ligne;
            const char *a = "COUT_FORK rss_mio=";
            while (*a) *p++ = *a++;
            p = entier(p, cumul_mio);
            a = " essai=";
            while (*a) *p++ = *a++;
            p = entier(p, essai);
            a = " fork_us=";
            while (*a) *p++ = *a++;
            p = entier(p, pid < 0 ? -1 : t1 - t0);
            *p++ = '\n';
            appel(1, 1, (long)ligne, p - ligne);
        }
    }

    // LE CONTRAT DE LA FRAME NON MISE A ZERO, VERIFIE OCTET PAR OCTET.
    //
    // `AddressSpace::duplicate` prend desormais ses frames par
    // `alloc_frame_a_recouvrir`, qui ne les met PAS a zero : elles sortent de
    // l'allocateur en portant ce qu'un autre processus y avait ecrit, et le
    // `copy_nonoverlapping` qui suit est la SEULE chose qui les rende
    // presentables. Si un jour cette copie devenait partielle, l'enfant
    // heriterait de la memoire d'un inconnu -- une fuite qu'aucun test
    // fonctionnel ne remarquerait, puisque le programme continuerait de
    // tourner.
    //
    // La verification porte sur TOUS les octets, pas sur un par page : c'est
    // exactement la difference entre « la page est la » et « la page est
    // celle du pere ». Quatre mebioctets suffisent a la faire -- ce qu'on
    // teste est une propriete du chemin, pas un volume.
    {
        const long octets = 4L * 1024L * 1024L;
        long base = appel6(9, 0, octets, 3, 0x22, -1, 0);
        if ((unsigned long)base >= (unsigned long)-4095L) {
            dis("COUT_FORK_COPIE ok=0 raison=mmap\n");
        } else {
            unsigned char *motif = (unsigned char *)base;
            for (long o = 0; o < octets; o++)
                motif[o] = (unsigned char)((o * 31 + (o >> 12)) & 0xff);

            long pid = appel(57, 0, 0, 0);
            if (pid == 0) {
                long erreurs = 0;
                for (long o = 0; o < octets; o++)
                    if (motif[o] != (unsigned char)((o * 31 + (o >> 12)) & 0xff))
                        erreurs++;
                char ligne[96];
                char *q = ligne;
                const char *r = erreurs == 0
                    ? "COUT_FORK_COPIE ok=1 erreurs="
                    : "COUT_FORK_COPIE ok=0 erreurs=";
                while (*r) *q++ = *r++;
                q = entier(q, erreurs);
                *q++ = '\n';
                appel(1, 1, (long)ligne, q - ligne);
                appel(60, erreurs == 0 ? 0 : 1, 0, 0);
                for (;;) {}
            }
            if (pid > 0)
                appel6(61, pid, 0, 0, 0, 0, 0);
            else
                dis("COUT_FORK_COPIE ok=0 raison=fork\n");
            appel(11 /* munmap */, base, octets, 0);
        }
    }

    // LE GESTE COMPLET : `fork` PUIS `execve`, COMME LE NAVIGATEUR LE FAIT.
    //
    // Les paliers ci-dessus mesurent la duplication seule. Mais
    // `Core::Process::spawn` enchaine immediatement un `execve` qui DETRUIT
    // l'espace tout juste duplique. Le pere parcourt donc deux fois sa taille
    // residente : une fois pour la recopier, une fois pour la rendre.
    //
    // Cette derniere mesure est la seule qui passe par `sys_execve` -- le
    // `exec` du shell, lui, construit la tache directement et n'emprunte pas
    // ce chemin. Sans elle, `PERF_EXECVE` ne serait jamais exerce.
    for (int essai = 1; essai <= ESSAIS; essai++) {
        long t0 = maintenant_us();
        long pid = appel(57 /* fork */, 0, 0, 0);
        if (pid == 0) {
            // TOUT SUR LA PILE, ET CE N'EST PAS UN DETAIL DE STYLE.
            //
            // Un `static char *argv[] = { chemin, 0 }` demande une
            // reinstallation `R_X86_64_RELATIVE` a la mise en place de
            // l'image. Ce binaire est `-static-pie -nostartfiles` : personne
            // ne rejoue ses relocations, et le pointeur reste tel que
            // l'editeur de liens l'a ecrit -- une adresse qui n'existe pas.
            // `execve` rendait alors EFAULT, silencieusement, et la mesure
            // s'arretait sur un enfant mort sans image.
            //
            // Construits sur la pile, les deux sont calcules a l'execution.
            char chemin[8];
            chemin[0] = '/'; chemin[1] = 's'; chemin[2] = 'o'; chemin[3] = 'r';
            chemin[4] = 't'; chemin[5] = 'i'; chemin[6] = 'e'; chemin[7] = 0;
            char *argv[2];
            argv[0] = chemin;
            argv[1] = 0;
            appel(59 /* execve */, (long)chemin, (long)argv, 0);
            appel(60, 3, 0, 0);
            for (;;) {}
        }
        long t1 = maintenant_us();
        if (pid > 0)
            appel6(61 /* wait4 */, pid, 0, 0, 0, 0, 0);
        long t2 = maintenant_us();

        char ligne[160];
        char *p = ligne;
        const char *a = "COUT_FORK_EXEC rss_mio=";
        while (*a) *p++ = *a++;
        p = entier(p, cumul_mio);
        a = " essai=";
        while (*a) *p++ = *a++;
        p = entier(p, essai);
        a = " fork_us=";
        while (*a) *p++ = *a++;
        p = entier(p, pid < 0 ? -1 : t1 - t0);
        a = " total_us=";
        while (*a) *p++ = *a++;
        p = entier(p, pid < 0 ? -1 : t2 - t0);
        *p++ = '\n';
        appel(1, 1, (long)ligne, p - ligne);
    }

    dis("=== COUT FORK FIN ===\n");
    appel(60, 0, 0, 0);
    for (;;) {}
}
