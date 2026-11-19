// Sonde ring 3 : un message IPC ne reveille QUE son destinataire.
//
// BOUCHAUD_REVEIL_CIBLE_V1 -- chaque ecriture dans un tube, un canal de
// socketpair ou un eventfd reveillait TOUTES les attentes poll/select/epoll
// du systeme. Ladybird tient une boucle d'evenements par fil et echange des
// milliers de messages IPC par seconde : chaque message reveillait chaque
// boucle, qui relisait ses descripteurs pour se rendormir (WHPX, 19/11 :
// `bo-navigateur` 15 s de temps noyau pour 0,3 s utilisateur).
//
// La sonde reproduit ce motif :
//   - INACTIFS processus dorment en poll(-1), chacun sur SON canal ;
//   - deux processus echangent ALLERS messages en ping-pong (poll + read +
//     write, comme une boucle d'evenements) ;
//   - a la fin, chaque inactif rapporte son temps CPU (getrusage SELF).
//
// Elle imprime :
//   REVEIL_CIBLE inactifs=N cpu_inactifs_us=... cpu_max_inactif_us=...
//                allers=... duree_ms=... allers_par_s=...
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <poll.h>
#include <time.h>
#include <sys/socket.h>
#include <sys/wait.h>

#define INACTIFS 12
#define ALLERS 20000

// Temps CPU du processus : CLOCK_PROCESS_CPUTIME_ID (present sur tous les
// noyaux compares ; `getrusage` n'existe pas encore partout).
static long cpu_us(void) {
    struct timespec t;
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &t);
    return t.tv_sec * 1000000L + t.tv_nsec / 1000L;
}

static long maintenant_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000L + t.tv_nsec / 1000000L;
}

// Attend un octet sur `fd` par poll(-1), comme une boucle d'evenements.
static int recoit(int fd, char *c) {
    struct pollfd p = {.fd = fd, .events = POLLIN};
    for (;;) {
        if (poll(&p, 1, -1) < 0) return -1;
        if (p.revents & POLLIN) return read(fd, c, 1) == 1 ? 0 : -1;
        if (p.revents & (POLLHUP | POLLERR)) return -1;
    }
}

int main(void) {
    int inactif[INACTIFS][2];
    pid_t pids[INACTIFS];
    for (int i = 0; i < INACTIFS; i++) {
        if (socketpair(AF_UNIX, SOCK_STREAM, 0, inactif[i]) != 0) return 1;
        pids[i] = fork();
        if (pids[i] == 0) {
            close(inactif[i][0]);
            char c;
            long debut = cpu_us();
            // Dort jusqu'au signal de fin du parent : UN message attendu.
            if (recoit(inactif[i][1], &c) != 0) _exit(2);
            long conso = cpu_us() - debut;
            if (write(inactif[i][1], &conso, sizeof conso) != sizeof conso) _exit(3);
            _exit(0);
        }
        close(inactif[i][1]);
    }

    int ping[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, ping) != 0) return 1;
    pid_t echo = fork();
    if (echo == 0) {
        close(ping[0]);
        char c;
        for (int n = 0; n < ALLERS; n++) {
            if (recoit(ping[1], &c) != 0) _exit(2);
            if (write(ping[1], &c, 1) != 1) _exit(3);
        }
        _exit(0);
    }
    close(ping[1]);

    // Laisser les inactifs s'endormir.
    struct timespec pause = {0, 300 * 1000000};
    nanosleep(&pause, 0);

    long t0 = maintenant_ms();
    char c = 'x';
    for (int n = 0; n < ALLERS; n++) {
        if (write(ping[0], &c, 1) != 1) return 4;
        if (recoit(ping[0], &c) != 0) return 5;
    }
    long duree = maintenant_ms() - t0;
    int statut;
    waitpid(echo, &statut, 0);

    long total = 0, pire = 0;
    for (int i = 0; i < INACTIFS; i++) {
        long conso = -1;
        char fin = 'q';
        if (write(inactif[i][0], &fin, 1) != 1) return 6;
        if (read(inactif[i][0], &conso, sizeof conso) != sizeof conso) return 7;
        waitpid(pids[i], &statut, 0);
        total += conso;
        if (conso > pire) pire = conso;
    }
    printf("REVEIL_CIBLE inactifs=%d cpu_inactifs_us=%ld cpu_max_inactif_us=%ld allers=%d duree_ms=%ld allers_par_s=%ld\n",
           INACTIFS, total, pire, ALLERS, duree, duree > 0 ? ALLERS * 1000L / duree : -1);
    fflush(stdout);
    return 0;
}
