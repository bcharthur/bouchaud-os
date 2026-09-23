/*
 * CUMUL-CPU : `/proc/stat` perd-il le temps d'un processus qui meurt ?
 *
 * # La question
 *
 * `proc_cpu_cumul()` somme `user_cpu_ns` et `kernel_cpu_ns` des taches
 * VIVANTES. Une tache qui meurt quitte donc la somme, et le temps qu'elle a
 * consomme disparait des compteurs.
 *
 * On pourrait croire le defaut sans consequence : `idle` est derive du temps
 * ecoule et domine, si bien que le TOTAL reste croissant et que la formule de
 * Ladybird ne divise jamais par un nombre negatif.
 *
 * Mais `user` et `system` sont lus pour eux-memes. Un moniteur qui trace la
 * charge utilisateur verrait une COURBE QUI RECULE a chaque fin de processus
 * -- et sur un navigateur, des processus meurent tout le temps : un onglet
 * qu'on ferme, un WebWorker qui finit, un ImageDecoder recycle.
 *
 * # La mesure
 *
 *   1. lire /proc/stat            -> avant
 *   2. un enfant brule du temps UTILISATEUR
 *   3. lire /proc/stat PENDANT    -> pendant  (user doit avoir monte)
 *   4. l'enfant meurt
 *   5. lire /proc/stat APRES      -> apres    (user recule-t-il ?)
 *
 * `apres < pendant` est la preuve. Elle ne se discute pas : un compteur
 * cumulatif ne recule jamais.
 *
 * Le pere ne brule RIEN : tout mouvement de `user` vient de l'enfant.
 */
static long appel6(long n,long a,long b,long c,long d,long e,long f){long r;register long r10 __asm__("r10")=d;register long r8 __asm__("r8")=e;register long r9 __asm__("r9")=f;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c),"r"(r10),"r"(r8),"r"(r9):"rcx","r11","memory");return r;}
static long appel(long n,long a,long b,long c){return appel6(n,a,b,c,0,0,0);}
static void dis(const char*s){long n=0;while(s[n])n++;appel(1,1,(long)s,n);}

struct ts { long sec; long nsec; };

/* Ecrit la premiere ligne de /proc/stat, prefixee. */
static void releve(const char *etiquette)
{
    long fd = appel6(257, -100, (long)"/proc/stat", 0, 0, 0, 0);
    if (fd < 0) { dis("CUMUL refuse\n"); return; }
    static char buf[512];
    long n = appel(0, fd, (long)buf, sizeof(buf) - 1);
    appel(3, fd, 0, 0);
    if (n <= 0) { dis("CUMUL vide\n"); return; }
    long k = 0;
    while (k < n && buf[k] != '\n') k++;
    dis("CUMUL ");
    dis(etiquette);
    dis(" ");
    appel(1, 1, (long)buf, k);
    dis("\n");
}

/* Ecrit le chemin /proc/<pid>/stat dans `sortie`. */
static void chemin_du_pid(char *sortie, long pid)
{
    const char *tete = "/proc/";
    int n = 0;
    while (tete[n]) { sortie[n] = tete[n]; n++; }
    char chiffres[12];
    int c = 0;
    if (pid == 0) chiffres[c++] = '0';
    while (pid > 0) { chiffres[c++] = (char)('0' + pid % 10); pid /= 10; }
    while (c > 0) sortie[n++] = chiffres[--c];
    const char *queue = "/stat";
    int q = 0;
    while (queue[q]) sortie[n++] = queue[q++];
    sortie[n] = 0;
}

void _start(void)
{
    /*
     * DEUX COMPTEURS EN VIS-A-VIS, et c'est ce qui tranche.
     *
     * Les essais precedents montraient un `user` global qui ne reculait
     * jamais a la mort de l'enfant. Deux lectures opposees restaient :
     *
     *   A  le temps de l'enfant SURVIT a sa mort -- pas de defaut ;
     *   B  le temps de l'enfant n'a JAMAIS ete compte -- defaut pire.
     *
     * Lire `/proc/<pid>/stat` de l'enfant EN MEME TEMPS que le global les
     * separe. Si l'enfant annonce deux cents jiffies et que le global ne
     * bouge pas de deux cents a sa mort, c'est A. S'il en annonce zero, c'est
     * B, et le defaut est ailleurs.
     */
    long r = appel6(56 /* fork */, 17, 0, 0, 0, 0, 0);
    if (r == 0) {
        volatile long s = 0;
        for (long t = 0; t < 400000000L; t++) s += t;
        appel(60, 0, 0, 0);
        for (;;) {}
    }
    static char chemin[32];
    chemin_du_pid(chemin, r);
    dis("CUMUL enfant_chemin ");
    dis(chemin);
    dis("\n");

    for (int i = 0; i < 20; i++) {
        static char etiquette[8];
        int n = 0;
        if (i >= 10) etiquette[n++] = (char)('0' + i / 10);
        etiquette[n++] = (char)('0' + i % 10);
        etiquette[n] = 0;
        releve(etiquette);
        /* Et la ligne de l'enfant, pour la meme seconde. */
        long fd = appel6(257, -100, (long)chemin, 0, 0, 0, 0);
        if (fd < 0) {
            dis("CUMUL enfant ");
            dis(etiquette);
            dis(" absent\n");
        } else {
            static char buf[512];
            long m = appel(0, fd, (long)buf, sizeof(buf) - 1);
            appel(3, fd, 0, 0);
            dis("CUMUL enfant ");
            dis(etiquette);
            dis(" ");
            long k = 0;
            while (k < m && buf[k] != '\n') k++;
            appel(1, 1, (long)buf, k);
            dis("\n");
        }
        struct ts d = { 3, 0 };
        appel(35, (long)&d, 0, 0);
    }
    long etat = 0;
    appel6(61, -1, (long)&etat, 0, 0, 0, 0);
    releve("final");
    dis("CUMUL FIN\n");
    appel(60, 0, 0, 0);
    for (;;) {}
}
