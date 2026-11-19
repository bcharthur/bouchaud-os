// Sonde ring 3 : les appels systeme qu'OpenJDK 21 demandait encore.
//
// BOUCHAUD_GETRUSAGE_V1 / BOUCHAUD_FLOCK_V1 -- HotSpot appelait getrusage
// (98), fchdir (81) et flock (73), que Bouchaud refusait. La sonde verifie
// leur SENS, pas seulement leur code de retour :
//
//   rusage   un calcul de ~0,3 s se voit dans ru_utime ; who invalide -> EINVAL
//   fchdir   le repertoire courant suit le descripteur ; fichier -> ENOTDIR
//   flock    un verrou exclusif bloque un AUTRE processus (EAGAIN avec
//            LOCK_NB) jusqu'a LOCK_UN ; il ne gene pas un verrou fcntl
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/file.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <pthread.h>
#include <time.h>

static volatile unsigned long puits;

static int rusage(void) {
    for (unsigned long i = 0; i < 300000000UL; i++) puits += i ^ (i >> 3);
    struct rusage r;
    if (getrusage(RUSAGE_SELF, &r) != 0) return 0;
    long us = r.ru_utime.tv_sec * 1000000L + r.ru_utime.tv_usec;
    struct rusage t;
    int fil = getrusage(RUSAGE_THREAD, &t) == 0;
    int invalide = getrusage(42, &t) == -1 && errno == EINVAL;
    printf("JVM_SYSCALLS rusage_utime_us=%ld\n", us);
    return us > 50000 && r.ru_utime.tv_usec < 1000000 && fil && invalide;
}

static void *brule(void *x) {
    (void)x;
    for (unsigned long i = 0; i < 200000000UL; i++) puits += i ^ (i >> 5);
    return 0;
}

static long utime_us(void) {
    struct rusage r;
    getrusage(RUSAGE_SELF, &r);
    return r.ru_utime.tv_sec * 1000000L + r.ru_utime.tv_usec;
}

// Le total du PROCESSUS garde le temps de ses fils morts : il ne recule
// jamais (revue #341 : une somme des seuls fils vivants reculait).
static int rusage_fils_morts(void) {
    long avant = utime_us(), pire_recul = 0, precedent = avant;
    for (int k = 0; k < 3; k++) {
        pthread_t t;
        if (pthread_create(&t, 0, brule, 0) != 0) return 0;
        pthread_join(t, 0);
        long maintenant = utime_us();
        if (precedent - maintenant > pire_recul) pire_recul = precedent - maintenant;
        precedent = maintenant;
    }
    printf("JVM_SYSCALLS rusage_fils_us=%ld recul_us=%ld\n", precedent - avant, pire_recul);
    return pire_recul == 0 && precedent - avant > 100000;
}

// Deux processus tiennent LOCK_SH et convertissent en LOCK_EX : l'un des
// deux doit l'obtenir (Linux retire d'abord l'ancien verrou). Sondage
// non bloquant avec echeance : un interblocage se voit en echec, pas en
// banc fige.
static int essaie_ex(int fd, int ms) {
    struct timespec pas = {0, 10 * 1000000};
    for (int i = 0; i < ms / 10; i++) {
        if (flock(fd, LOCK_EX | LOCK_NB) == 0) return 1;
        nanosleep(&pas, 0);
    }
    return 0;
}

static int flock_conversion(void) {
    const char *chemin = "/tmp/verrou-conversion";
    int fd = open(chemin, O_CREAT | O_RDWR, 0644);
    int pret[2], go[2];
    pipe(pret);
    pipe(go);
    pid_t pid = fork();
    if (pid == 0) {
        int g = open(chemin, O_RDWR);
        char c = 1;
        if (flock(g, LOCK_SH) != 0) _exit(2);
        write(pret[1], &c, 1);
        read(go[0], &c, 1);
        int ok = essaie_ex(g, 4000);
        if (ok) flock(g, LOCK_UN);
        _exit(ok ? 0 : 1);
    }
    char c;
    flock(fd, LOCK_SH);
    read(pret[0], &c, 1);
    write(go[1], &c, 1);
    int ok = essaie_ex(fd, 4000);
    if (ok) {
        struct timespec pause = {0, 100 * 1000000};
        nanosleep(&pause, 0);
        flock(fd, LOCK_UN);
    }
    int statut = 0;
    waitpid(pid, &statut, 0);
    close(fd);
    int fils_ok = WIFEXITED(statut) && WEXITSTATUS(statut) == 0;
    printf("JVM_SYSCALLS conversion pere=%d fils=%d\n", ok, fils_ok);
    return ok && fils_ok;
}

static int fchdir_(void) {
    int d = open("/usr", O_RDONLY | O_DIRECTORY);
    if (d < 0) return 0;
    char cwd[256] = {0};
    int ok = fchdir(d) == 0 && getcwd(cwd, sizeof cwd) && strcmp(cwd, "/usr") == 0;
    close(d);
    int f = open("/jvm-syscalls-probe", O_RDONLY);
    int pas_rep = fchdir(f) == -1 && errno == ENOTDIR;
    close(f);
    int mauvais = fchdir(999) == -1 && errno == EBADF;
    chdir("/");
    return ok && pas_rep && mauvais;
}

static int flock_(void) {
    const char *chemin = "/tmp/verrou-flock";
    int fd = open(chemin, O_CREAT | O_RDWR, 0644);
    if (fd < 0 || flock(fd, LOCK_EX) != 0) return 0;
    // Deux tubes : un par sens. Avec un seul, le fils pourrait relire son
    // propre octet.
    int vers_pere[2], vers_fils[2];
    pipe(vers_pere);
    pipe(vers_fils);
    pid_t pid = fork();
    if (pid == 0) {
        int g = open(chemin, O_RDWR);
        int bloque = flock(g, LOCK_EX | LOCK_NB) == -1 && errno == EWOULDBLOCK;
        // Independance : un verrou fcntl sur le meme fichier passe.
        struct flock fl = {.l_type = F_WRLCK, .l_whence = SEEK_SET, .l_start = 0, .l_len = 0};
        int fcntl_libre = fcntl(g, F_SETLK, &fl) == 0;
        char c = 1;
        write(vers_pere[1], &c, 1);
        read(vers_fils[0], &c, 1);  // attend que le pere relache
        // Le pere a relache : le verrou s'obtient, en attente bloquante.
        int obtenu = flock(g, LOCK_EX) == 0;
        _exit(bloque && fcntl_libre && obtenu ? 0 : 1);
    }
    char c;
    read(vers_pere[0], &c, 1);
    // Partage puis exclusif par le meme processus : une conversion, pas un conflit.
    int conversion = flock(fd, LOCK_SH) == 0 && flock(fd, LOCK_EX) == 0;
    flock(fd, LOCK_UN);
    c = 2;
    write(vers_fils[1], &c, 1);
    int statut = 0;
    waitpid(pid, &statut, 0);
    int invalide = flock(fd, 0) == -1 && errno == EINVAL;
    close(fd);
    return conversion && invalide && WIFEXITED(statut) && WEXITSTATUS(statut) == 0;
}

int main(void) {
    int a = rusage() && rusage_fils_morts(), b = fchdir_(), c = flock_() && flock_conversion();
    printf("JVM_SYSCALLS_FIN rusage=%d fchdir=%d flock=%d\n", a, b, c);
    fflush(stdout);
    return 0;
}
