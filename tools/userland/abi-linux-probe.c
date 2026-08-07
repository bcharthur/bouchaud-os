// Un binaire Linux ordinaire, lie dynamiquement a la glibc, tourne-t-il ?
//
// C'est la question qui decide de la suite. Bouchaud OS expose l'ABI Linux
// x86-64 ; si un binaire du monde reel s'y execute sans recompilation, alors
// amener un moteur web n'est pas le reecrire — c'est honorer les appels que son
// binaire emet. C'est la methode du linuxulator de FreeBSD, de WSL1 et de
// gVisor, et elle demande incomparablement moins de travail qu'un portage.
//
// La sonde n'exerce pas des appels au hasard : elle exerce ce dont la **glibc**
// a besoin pour demarrer et pour tenir, dans l'ordre ou elle s'en sert. Chaque
// etape franchie est une brique de moins a porter ; la premiere qui lache nomme
// ce qu'il faut ecrire ensuite.

#define _GNU_SOURCE
#include <dlfcn.h>
#include <locale.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/utsname.h>
#include <time.h>
#include <unistd.h>

static int echecs = 0;
static int reussites = 0;

static void verifie(const char *nom, int condition, const char *detail)
{
    if (condition) {
        reussites++;
        printf("  ok    %s\n", nom);
    } else {
        echecs++;
        printf("  ECHEC %s%s%s\n", nom, detail ? " : " : "", detail ? detail : "");
    }
}

static void *fil_travail(void *donnee)
{
    int *compteur = donnee;
    for (int i = 0; i < 1000; i++)
        __sync_fetch_and_add(compteur, 1);
    return donnee;
}

int main(void)
{
    printf("abi-linux-probe : un binaire glibc dynamique sous Bouchaud OS\n\n");

    // Si cette ligne s'imprime, l'essentiel est acquis : le noyau a lu
    // `PT_INTERP`, charge `ld-linux-x86-64.so.2`, qui a mappe la glibc, resolu
    // les symboles, monte le TLS et appele `main`. C'est la partie difficile.
    verifie("le chargeur dynamique a rendu la main a main()", 1, NULL);

    struct utsname noms;
    verifie("uname", uname(&noms) == 0, NULL);
    printf("        systeme : %s %s\n", noms.sysname, noms.release);
    verifie("getpid rend un identifiant plausible", getpid() > 0, NULL);
    verifie("sysconf(PAGESIZE)", sysconf(_SC_PAGESIZE) == 4096, NULL);

    struct timespec maintenant;
    verifie("clock_gettime(MONOTONIC)",
            clock_gettime(CLOCK_MONOTONIC, &maintenant) == 0, NULL);

    // L'allocateur : `brk` pour les petites tailles, `mmap` au-dela d'un seuil.
    // Les deux chemins comptent — un moteur web alloue par millions.
    char *petit = malloc(64);
    verifie("malloc court (brk)", petit != NULL, NULL);
    if (petit) { strcpy(petit, "brk"); free(petit); }

    char *grand = malloc(4 * 1024 * 1024);
    verifie("malloc long (mmap)", grand != NULL, NULL);
    if (grand) {
        memset(grand, 0x5A, 4 * 1024 * 1024);
        verifie("la zone allouee est reellement ecrivable",
                grand[4 * 1024 * 1024 - 1] == 0x5A, NULL);
        free(grand);
    }

    char *bouge = malloc(1024);
    if (bouge) {
        strcpy(bouge, "avant");
        bouge = realloc(bouge, 512 * 1024);
        verifie("realloc conserve le contenu",
                bouge && strcmp(bouge, "avant") == 0, bouge);
        free(bouge);
    }

    // Les fils : `clone`, `set_robust_list`, `futex`, une zone TLS par fil.
    // Un moteur web est multi-fils de bout en bout.
    int compteur = 0;
    pthread_t fil;
    int cree = pthread_create(&fil, NULL, fil_travail, &compteur);
    verifie("pthread_create", cree == 0, cree ? strerror(cree) : NULL);
    if (cree == 0) {
        void *rendu = NULL;
        verifie("pthread_join", pthread_join(fil, &rendu) == 0, NULL);
        verifie("le fil a bien travaille", compteur == 1000, NULL);
    }

    pthread_mutex_t verrou = PTHREAD_MUTEX_INITIALIZER;
    verifie("pthread_mutex_lock", pthread_mutex_lock(&verrou) == 0, NULL);
    verifie("pthread_mutex_unlock", pthread_mutex_unlock(&verrou) == 0, NULL);

    // `__thread` passe par la disposition TLS de la glibc, qui n'est pas celle
    // de musl.
    static __thread int local = 42;
    local += 1;
    verifie("variable a stockage local de fil", local == 43, NULL);

    // `dlopen` : c'est ainsi qu'un moteur web charge ses greffons et ses
    // backends. Sans lui, WPE ne trouverait pas le sien.
    void *poignee = dlopen("libc.so.6", RTLD_LAZY);
    verifie("dlopen d'une bibliotheque deja chargee", poignee != NULL,
            poignee ? NULL : dlerror());
    if (poignee) {
        verifie("dlsym retrouve un symbole", dlsym(poignee, "strlen") != NULL, NULL);
        dlclose(poignee);
    }

    verifie("setlocale(C)", setlocale(LC_ALL, "C") != NULL, NULL);

    FILE *fichier = fopen("/tmp/abi-linux-essai", "w+");
    verifie("fopen en ecriture", fichier != NULL, NULL);
    if (fichier) {
        fprintf(fichier, "ecrit par la glibc\n");
        fflush(fichier);
        rewind(fichier);
        char ligne[64] = {0};
        verifie("relecture par la glibc",
                fgets(ligne, sizeof ligne, fichier) != NULL
                && strncmp(ligne, "ecrit par la glibc", 18) == 0, ligne);
        fclose(fichier);
        unlink("/tmp/abi-linux-essai");
    }

    printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n",
           echecs, reussites);
    return echecs == 0 ? 0 : 1;
}
