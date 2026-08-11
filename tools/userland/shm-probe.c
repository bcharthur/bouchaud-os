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
#include <sys/socket.h>
#include <sys/uio.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/sysinfo.h>
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
static const char PASSE[] = "tampon cree par le fils, projete par le pere";

// Envoie un descripteur sur un socket, facon `SCM_RIGHTS`.
static int envoie_descripteur(int socket, int fd)
{
    char octet = 'x';
    struct iovec iov = { .iov_base = &octet, .iov_len = 1 };
    char place[CMSG_SPACE(sizeof(int))];
    memset(place, 0, sizeof place);

    struct msghdr message;
    memset(&message, 0, sizeof message);
    message.msg_iov = &iov;
    message.msg_iovlen = 1;
    message.msg_control = place;
    message.msg_controllen = sizeof place;

    struct cmsghdr *entete = CMSG_FIRSTHDR(&message);
    entete->cmsg_level = SOL_SOCKET;
    entete->cmsg_type = SCM_RIGHTS;
    entete->cmsg_len = CMSG_LEN(sizeof(int));
    memcpy(CMSG_DATA(entete), &fd, sizeof(int));

    return sendmsg(socket, &message, 0) >= 0 ? 0 : -1;
}

// Recoit un descripteur, ou -1.
static int recoit_descripteur(int socket)
{
    char octet = 0;
    struct iovec iov = { .iov_base = &octet, .iov_len = 1 };
    char place[CMSG_SPACE(sizeof(int))];
    memset(place, 0, sizeof place);

    struct msghdr message;
    memset(&message, 0, sizeof message);
    message.msg_iov = &iov;
    message.msg_iovlen = 1;
    message.msg_control = place;
    message.msg_controllen = sizeof place;

    if (recvmsg(socket, &message, 0) < 0)
        return -1;
    struct cmsghdr *entete = CMSG_FIRSTHDR(&message);
    if (!entete || entete->cmsg_level != SOL_SOCKET
            || entete->cmsg_type != SCM_RIGHTS)
        return -1;
    int recu = -1;
    memcpy(&recu, CMSG_DATA(entete), sizeof(int));
    return recu;
}

// Le fils cree un tampon, y ecrit, et **passe le descripteur** au pere par un
// socket. Le pere le projette et doit y voir l'ecriture : partager de la
// memoire ne sert a rien si l'on ne peut pas confier ce qui la designe.
static void verifie_passage(void)
{
    int paire[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, paire) != 0) {
        verifie("socketpair pour le passage", 0, NULL);
        return;
    }
    verifie("socketpair pour le passage", 1, NULL);

    pid_t enfant = fork();
    if (enfant < 0) {
        verifie("fork pour le passage", 0, NULL);
        return;
    }

    if (enfant == 0) {
        close(paire[0]);
        int tampon = memfd("passe-par-le-fils");
        if (tampon < 0 || ftruncate(tampon, 4096) != 0)
            _exit(2);
        char *zone = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED,
                          tampon, 0);
        if (zone == MAP_FAILED)
            _exit(3);
        strcpy(zone, PASSE);
        munmap(zone, 4096);
        if (envoie_descripteur(paire[1], tampon) != 0)
            _exit(4);
        close(tampon);
        close(paire[1]);
        _exit(0);
    }

    close(paire[1]);
    int recu = recoit_descripteur(paire[0]);
    verifie("un descripteur traverse le socket", recu >= 0, NULL);

    if (recu >= 0) {
        char *zone = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED,
                          recu, 0);
        verifie("le descripteur recu se projette", zone != MAP_FAILED, NULL);
        if (zone != MAP_FAILED) {
            // Le pere n'a jamais ouvert ce tampon : il ne l'a recu que par le
            // socket. S'il y lit ce que le fils a ecrit, le descripteur designe
            // bien la meme memoire.
            verifie("et porte ce que le fils y avait ecrit",
                    strcmp(zone, PASSE) == 0, zone);
            munmap(zone, 4096);
        }
        close(recu);
    }

    int statut = 0;
    waitpid(enfant, &statut, 0);
    verifie("le fils qui a passe le descripteur a fini normalement",
            WIFEXITED(statut) && WEXITSTATUS(statut) == 0, NULL);
    close(paire[0]);
}

// Memoire physique encore libre, en kibioctets.
//
// `sysinfo` interroge l'allocateur de frames en direct — contrairement a
// `/proc/meminfo`, qui n'est qu'un instantane pris au demarrage. C'est la seule
// observation qui puisse distinguer « le descripteur a disparu » de « les pages
// sont revenues », et c'est la seconde qui nous interesse.
static unsigned long libre_kb(void)
{
    struct sysinfo info;
    if (sysinfo(&info) != 0)
        return 0;
    return (unsigned long)(info.freeram / 1024);
}

#define CYCLES 1000
#define PAGES_PAR_CYCLE 8

// Le cycle de vie complet d'une surface partagee, mille fois.
//
// C'est le scenario d'un compositeur qui recree son tampon a chaque
// redimensionnement. Chaque tour alloue huit pages, les projette, y ecrit, les
// relit, puis rend tout. Si le cache de pages ne rendait rien — ce qui etait le
// cas —, mille tours coutent 32 MiB qui ne reviennent jamais, et la centieme
// fenetre redimensionnee tue la machine.
//
// Le test ne se contente pas de verifier que `close` rend 0 : il compare la
// memoire physique libre avant et apres. C'est la seule chose qui prouve que
// les frames sont revenues.
static void verifie_eviction(void)
{
    printf("  cycle de vie : %d creations/destructions de surface\n", CYCLES);

    // Un tour a blanc d'abord : la premiere iteration peut faire grossir des
    // structures du noyau (table de nœuds, vecteur du cache) qui ne
    // retrecissent pas. Les mesurer comme une fuite serait une fausse alerte.
    for (int i = 0; i < 4; i++) {
        int fd = memfd("prechauffage");
        if (fd < 0)
            return;
        if (ftruncate(fd, PAGES_PAR_CYCLE * 4096) != 0) {
            close(fd);
            return;
        }
        char *z = mmap(NULL, PAGES_PAR_CYCLE * 4096, PROT_READ | PROT_WRITE,
                       MAP_SHARED, fd, 0);
        if (z != MAP_FAILED)
            munmap(z, PAGES_PAR_CYCLE * 4096);
        close(fd);
    }

    unsigned long avant = libre_kb();
    verifie("sysinfo rend la memoire libre", avant > 0, NULL);
    if (avant == 0)
        return;

    int rates = 0;
    int perdus = 0;
    for (int tour = 0; tour < CYCLES; tour++) {
        int fd = memfd("surface");
        if (fd < 0) {
            rates++;
            break;
        }
        if (ftruncate(fd, PAGES_PAR_CYCLE * 4096) != 0) {
            rates++;
            close(fd);
            break;
        }
        char *zone = mmap(NULL, PAGES_PAR_CYCLE * 4096, PROT_READ | PROT_WRITE,
                          MAP_SHARED, fd, 0);
        if (zone == MAP_FAILED) {
            rates++;
            close(fd);
            break;
        }
        // Toucher chaque page : sans cela le cache pourrait n'en materialiser
        // aucune, et le test ne prouverait rien.
        for (int page = 0; page < PAGES_PAR_CYCLE; page++)
            zone[page * 4096] = (char)(tour + page);
        for (int page = 0; page < PAGES_PAR_CYCLE; page++) {
            if (zone[page * 4096] != (char)(tour + page))
                perdus++;
        }
        munmap(zone, PAGES_PAR_CYCLE * 4096);
        close(fd);
    }

    verifie("les mille cycles se sont tous ouverts", rates == 0, NULL);
    verifie("chaque page relue porte ce qu'on y a ecrit", perdus == 0, NULL);

    unsigned long apres = libre_kb();
    // Le budget total qui aurait fui sans eviction.
    unsigned long fuite_totale = (unsigned long)CYCLES * PAGES_PAR_CYCLE * 4;
    // On tolere un dixieme de ce budget : le noyau alloue aussi pour ses
    // propres structures, et exiger l'egalite stricte ferait un test instable —
    // ce qui est pire qu'un test absent, parce qu'il apprend a ignorer les
    // echecs.
    unsigned long seuil = fuite_totale / 10;
    unsigned long consomme = apres < avant ? avant - apres : 0;

    char detail[128];
    snprintf(detail, sizeof detail,
             "%lu KiB avant, %lu KiB apres (%lu consommes, budget de fuite %lu)",
             avant, apres, consomme, fuite_totale);
    verifie("la memoire physique revient apres chaque surface",
            consomme < seuil, detail);

    // Et la contre-epreuve : un mappage encore vivant ne doit **pas** etre
    // evince par la fermeture de son descripteur. Liberer la, ce serait
    // corrompre la memoire d'un processus qui travaille.
    int fd = memfd("survivante");
    if (fd >= 0 && ftruncate(fd, 4096) == 0) {
        char *zone = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        if (zone != MAP_FAILED) {
            strcpy(zone, "toujours la");
            close(fd); // le descripteur part, le mappage reste
            verifie("un mappage vivant survit a la fermeture du descripteur",
                    strcmp(zone, "toujours la") == 0, zone);
            munmap(zone, 4096);
        } else {
            close(fd);
        }
    }
}

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

    // 6. Le passage de descripteurs — l'autre moitie du multi-processus.
    verifie_passage();

    // 7. Le cycle de vie : partager ne suffit pas, il faut aussi rendre.
    verifie_eviction();

    printf("RESULTAT : %d verification(s) en echec (%d passees)\n",
           echecs, reussites);
    return echecs == 0 ? 0 : 1;
}
