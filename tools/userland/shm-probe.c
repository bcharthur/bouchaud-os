// Memoire partagee : ce sur quoi repose un moteur web multi-processus.
//
// WebKit et Chromium ne rendent pas une page dans un seul processus. Celui qui
// decode une image, celui qui met en page, celui qui compose et celui qui parle
// au reseau sont distincts — c'est ce qui fait qu'un site qui plante n'emporte
// pas le navigateur. Ils se passent leurs tampons **sans les copier** : un
// descripteur anonyme, mappe en `MAP_SHARED` de part et d'autre, et les deux
// processus ecrivent dans les memes pages physiques.
//
// Cette sonde etablit exactement cela, et rien de plus :
//
//   1. `memfd_create` rend un descripteur sans nom dans l'arborescence ;
//   2. `ftruncate` lui donne une taille ;
//   3. `mmap(MAP_SHARED)` le projette ;
//   4. un `fork`, et le fils ecrit dans sa projection ;
//   5. le pere relit la sienne et doit y voir ce que le fils a ecrit.
//
// Si la derniere etape echoue, les deux processus ont chacun leur copie : le
// partage est un mensonge, et aucun moteur multi-processus ne fonctionnera.
//
// `/dev/shm` est verifie au passage : c'est la que `shm_open` cree ses
// segments, et son absence fait echouer toute memoire partagee nommee.

#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
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

#ifndef SYS_memfd_create
#define SYS_memfd_create 319
#endif

static int memfd(const char *nom)
{
    return (int)syscall(SYS_memfd_create, nom, 1 /* MFD_CLOEXEC */);
}

#define TAILLE 8192
static const char TEMOIN[] = "ecrit par le fils, lu par le pere";

int main(void)
{
    printf("shm-probe: memoire partagee entre processus\n");

    // 1. Le descripteur anonyme.
    int fd = memfd("tampon-de-rendu");
    verifie("memfd_create rend un descripteur", fd >= 0, NULL);
    if (fd < 0) {
        printf("RESULTAT : %d verification(s) en echec (%d passees)\n",
               echecs, reussites);
        return 1;
    }

    // Il ne doit apparaitre nulle part dans l'arborescence : c'est ce qui le
    // distingue d'un fichier ordinaire, et ce qui le rend sur.
    struct stat etat;
    verifie("il a une taille (nulle au depart)", fstat(fd, &etat) == 0, NULL);
    verifie("et cette taille est bien nulle", etat.st_size == 0, NULL);

    verifie("ftruncate lui donne sa taille", ftruncate(fd, TAILLE) == 0, NULL);
    verifie("la taille est prise en compte",
            fstat(fd, &etat) == 0 && etat.st_size == TAILLE, NULL);

    // 2. La projection partagee.
    char *zone = mmap(NULL, TAILLE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    verifie("mmap MAP_SHARED reussit", zone != MAP_FAILED, NULL);
    if (zone == MAP_FAILED) {
        printf("RESULTAT : %d verification(s) en echec (%d passees)\n",
               echecs, reussites);
        return 1;
    }

    memset(zone, 0, TAILLE);
    strcpy(zone, "ecrit par le pere");
    verifie("le pere ecrit dans sa projection",
            strcmp(zone, "ecrit par le pere") == 0, zone);

    // 3. Le partage lui-meme.
    pid_t enfant = fork();
    verifie("fork reussit", enfant >= 0, NULL);
    if (enfant == 0) {
        // Le fils projette **le meme** descripteur, herite par le fork.
        char *sienne = mmap(NULL, TAILLE, PROT_READ | PROT_WRITE,
                            MAP_SHARED, fd, 0);
        if (sienne == MAP_FAILED)
            _exit(2);
        // Il doit d'abord voir ce que le pere a ecrit.
        if (strcmp(sienne, "ecrit par le pere") != 0)
            _exit(3);
        strcpy(sienne, TEMOIN);
        // Et une ecriture plus loin dans la zone, pour verifier que le partage
        // ne porte pas que sur la premiere page.
        strcpy(sienne + 4096, "seconde page");
        _exit(0);
    }

    int statut = 0;
    waitpid(enfant, &statut, 0);
    verifie("le fils s'est termine normalement",
            WIFEXITED(statut) && WEXITSTATUS(statut) == 0, NULL);
    if (WIFEXITED(statut) && WEXITSTATUS(statut) == 3)
        printf("  (le fils ne voyait pas l'ecriture du pere)\n");

    // 4. Le pere doit voir l'ecriture du fils — sans avoir rien relu.
    verifie("le pere voit ce que le fils a ecrit",
            strcmp(zone, TEMOIN) == 0, zone);
    verifie("le partage porte au-dela de la premiere page",
            strcmp(zone + 4096, "seconde page") == 0, zone + 4096);

    munmap(zone, TAILLE);
    close(fd);

    // 5. `/dev/shm`, ou `shm_open` depose ses segments nommes.
    verifie("/dev/shm existe",
            stat("/dev/shm", &etat) == 0 && S_ISDIR(etat.st_mode), NULL);
    int nomme = open("/dev/shm/essai", O_RDWR | O_CREAT, 0600);
    verifie("on peut y creer un segment", nomme >= 0, NULL);
    if (nomme >= 0) {
        verifie("et le dimensionner", ftruncate(nomme, 4096) == 0, NULL);
        char *m = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, nomme, 0);
        verifie("et le projeter en partage", m != MAP_FAILED, NULL);
        if (m != MAP_FAILED) {
            strcpy(m, "segment nomme");
            verifie("il retient ce qu'on y ecrit",
                    strcmp(m, "segment nomme") == 0, m);
            munmap(m, 4096);
        }
        close(nomme);
        unlink("/dev/shm/essai");
    }

    printf("RESULTAT : %d verification(s) en echec (%d passees)\n",
           echecs, reussites);
    return echecs == 0 ? 0 : 1;
}
