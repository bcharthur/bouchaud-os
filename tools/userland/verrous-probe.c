/*
 * Sonde des verrous d'enregistrement POSIX — la primitive dont depend SQLite,
 * donc IndexedDB et le stockage SQL du navigateur.
 *
 * Ce n'est pas un test academique. `sqlite3` (unixCheckReservedLock,
 * unixLock, unixUnlock dans os_unix.c) fonde toute sa concurrence sur
 * `fcntl(F_SETLK)` et `fcntl(F_GETLK)`, et l'algorithme est sensible a la
 * VALEUR RENVOYEE DANS LA STRUCTURE, pas seulement au code de retour :
 *
 *     lock.l_type = F_WRLCK;
 *     fcntl(fd, F_GETLK, &lock);
 *     if (lock.l_type != F_UNLCK) reserved = 1;   // « un autre me bloque »
 *
 * Un noyau qui repond « 0 » sans jamais ecrire dans `lock` laisse donc
 * `l_type` a F_WRLCK : SQLite conclut qu'un autre processus tient un verrou
 * RESERVED et rend SQLITE_BUSY sur toute transaction en ecriture. Le defaut
 * ne se voit pas dans le code de retour ; il se voit ici.
 *
 *   musl-gcc -O2 -static-pie verrous-probe.c -o verrous-probe
 *   (ou via ./build.sh musl)
 */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-52s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

static const char *nom_type(short type)
{
    switch (type) {
    case F_RDLCK: return "F_RDLCK";
    case F_WRLCK: return "F_WRLCK";
    case F_UNLCK: return "F_UNLCK";
    default:      return "?";
    }
}

/* Prepare un `struct flock` sur une plage. */
static void remplis(struct flock *v, short type, off_t debut, off_t longueur)
{
    memset(v, 0, sizeof(*v));
    v->l_type = type;
    v->l_whence = SEEK_SET;
    v->l_start = debut;
    v->l_len = longueur;
}

int main(void)
{
    printf("Sonde verrous POSIX — fcntl(F_SETLK/F_GETLK), prerequis SQLite\n\n");

    const char *chemin = "/tmp/verrous-probe.db";
    int fd = open(chemin, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        printf("  open(%s) impossible : %s\n", chemin, strerror(errno));
        printf("VERROUS_POSIX_FAIL echecs=1\n");
        return 1;
    }
    if (write(fd, "bouchaud", 8) != 8) {
        printf("  write initial impossible : %s\n", strerror(errno));
        printf("VERROUS_POSIX_FAIL echecs=1\n");
        return 1;
    }

    printf("[fichier libre]\n");

    /* 1. Personne ne tient rien : F_GETLK doit REECRIRE l_type a F_UNLCK. */
    struct flock v;
    remplis(&v, F_WRLCK, 0, 0);
    int rc = fcntl(fd, F_GETLK, &v);
    verifie("F_GETLK sur fichier libre renvoie 0", rc == 0);
    printf("      l_type rendu = %s (attendu F_UNLCK)\n", nom_type(v.l_type));
    verifie("F_GETLK ecrit l_type = F_UNLCK", v.l_type == F_UNLCK);

    /* 2. Prendre un verrou en ecriture sur tout le fichier. */
    printf("\n[prise de verrou]\n");
    remplis(&v, F_WRLCK, 0, 0);
    rc = fcntl(fd, F_SETLK, &v);
    verifie("F_SETLK F_WRLCK sur fichier libre reussit", rc == 0);

    /* 3. Le meme processus interroge : POSIX veut F_UNLCK, car un processus
     *    ne se bloque jamais lui-meme. */
    remplis(&v, F_WRLCK, 0, 0);
    rc = fcntl(fd, F_GETLK, &v);
    verifie("F_GETLK par le proprietaire renvoie 0", rc == 0);
    printf("      l_type rendu = %s (attendu F_UNLCK : on ne se bloque pas soi-meme)\n",
        nom_type(v.l_type));
    verifie("F_GETLK par le proprietaire ecrit F_UNLCK", v.l_type == F_UNLCK);

    /* 4. Un autre processus doit, lui, etre bloque et le voir. */
    printf("\n[conflit entre processus]\n");
    int tube[2];
    if (pipe(tube) != 0) {
        printf("  pipe() impossible : %s\n", strerror(errno));
        printf("VERROUS_POSIX_FAIL echecs=1\n");
        return 1;
    }

    pid_t parent = getpid();
    pid_t enfant = fork();
    if (enfant < 0) {
        printf("  fork() impossible : %s\n", strerror(errno));
        printf("VERROUS_POSIX_FAIL echecs=1\n");
        return 1;
    }

    if (enfant == 0) {
        close(tube[0]);
        int resultat[3];

        int fd2 = open(chemin, O_RDWR);
        struct flock w;

        /* F_SETLK non bloquant doit ECHOUER avec EACCES ou EAGAIN. */
        remplis(&w, F_WRLCK, 0, 0);
        int pose = fcntl(fd2, F_SETLK, &w);
        resultat[0] = (pose == -1 && (errno == EACCES || errno == EAGAIN)) ? 1 : 0;

        /* F_GETLK doit rapporter le verrou du parent. */
        remplis(&w, F_WRLCK, 0, 0);
        fcntl(fd2, F_GETLK, &w);
        resultat[1] = (w.l_type == F_WRLCK) ? 1 : 0;
        resultat[2] = (int)w.l_pid;

        ssize_t ignore = write(tube[1], resultat, sizeof(resultat));
        (void)ignore;
        close(tube[1]);
        _exit(0);
    }

    close(tube[1]);
    int resultat[3] = { -1, -1, -1 };
    ssize_t lus = read(tube[0], resultat, sizeof(resultat));
    close(tube[0]);
    int statut = 0;
    waitpid(enfant, &statut, 0);

    verifie("l'enfant a pu rendre son verdict", lus == (ssize_t)sizeof(resultat));
    verifie("F_SETLK d'un autre processus echoue (EACCES/EAGAIN)", resultat[0] == 1);
    verifie("F_GETLK d'un autre processus voit F_WRLCK", resultat[1] == 1);
    printf("      l_pid rapporte = %d (parent = %d)\n", resultat[2], (int)parent);
    verifie("F_GETLK rapporte le pid du detenteur", resultat[2] == (int)parent);

    /* 5. Relacher, puis verifier qu'un autre processus peut prendre. */
    printf("\n[liberation]\n");
    remplis(&v, F_UNLCK, 0, 0);
    rc = fcntl(fd, F_SETLK, &v);
    verifie("F_SETLK F_UNLCK reussit", rc == 0);

    pid_t second = fork();
    if (second == 0) {
        int fd3 = open(chemin, O_RDWR);
        struct flock w;
        remplis(&w, F_WRLCK, 0, 0);
        _exit(fcntl(fd3, F_SETLK, &w) == 0 ? 0 : 1);
    }
    statut = 0;
    waitpid(second, &statut, 0);
    verifie("apres liberation, un autre processus obtient le verrou",
        WIFEXITED(statut) && WEXITSTATUS(statut) == 0);

    /* 6. Le verrou d'un processus mort doit disparaitre avec lui. */
    printf("\n[verrou d'un processus disparu]\n");
    remplis(&v, F_WRLCK, 0, 0);
    rc = fcntl(fd, F_SETLK, &v);
    verifie("le parent reprend le verrou apres la mort du second", rc == 0);

    /* 7. Verrous par plage : deux plages disjointes coexistent. */
    printf("\n[plages disjointes]\n");
    remplis(&v, F_UNLCK, 0, 0);
    fcntl(fd, F_SETLK, &v);
    remplis(&v, F_WRLCK, 0, 4);
    verifie("verrou sur [0,4) accepte", fcntl(fd, F_SETLK, &v) == 0);
    remplis(&v, F_WRLCK, 4, 4);
    rc = fcntl(fd, F_GETLK, &v);
    printf("      l_type sur [4,8) = %s (attendu F_UNLCK, plage disjointe)\n",
        nom_type(v.l_type));
    verifie("F_GETLK sur une plage disjointe rend F_UNLCK", rc == 0 && v.l_type == F_UNLCK);

    close(fd);
    unlink(chemin);

    printf("\nRESULTAT : %d verification(s) en echec\n", echecs);
    if (echecs == 0)
        printf("VERROUS_POSIX_OK\n");
    else
        printf("VERROUS_POSIX_FAIL echecs=%d\n", echecs);
    return echecs == 0 ? 0 : 1;
}
