/*
 * Sonde d'heritage de descripteur a travers `execve` — le mecanisme par lequel
 * le BrowserHost Ladybird donne a chaque service sa socket IPC.
 *
 * Upstream, `Core::Process::spawn` cree une paire de sockets, laisse une
 * extremite ouverte a travers l'exec, et l'annonce au programme charge par la
 * variable `SOCKET_TAKEOVER`. Le nouvel executable la reprend avec
 * `IPC::take_over_accepted_client_from_system_server`. Rien de tout cela ne
 * fonctionne si le noyau ne garantit pas trois choses :
 *
 *   1. un descripteur SANS FD_CLOEXEC survit a `execve` ;
 *   2. il reste une SOCKET apres l'exec — `fstat` doit dire S_IFSOCK, pas un
 *      fichier ordinaire (regression deja rencontree sur Bouchaud) ;
 *   3. l'environnement passe a `execve` arrive intact au nouveau programme,
 *      c'est par la que voyage le numero du descripteur.
 *
 * Et symetriquement : un descripteur AVEC FD_CLOEXEC doit disparaitre. Sans
 * cela, chaque service heriterait des sockets de tous les autres.
 *
 * Le programme se relance lui-meme en enfant, reconnu par BOUCHAUD_TAKEOVER_FD.
 *
 *   musl-gcc -O2 -static-pie exec-fd-probe.c -o exec-fd-probe
 *   (ou via ./build.sh musl)
 */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static int echecs = 0;

static void verifie(const char *quoi, int condition)
{
    printf("  %-52s %s\n", quoi, condition ? "ok" : "ECHEC");
    if (!condition)
        echecs++;
}

/* --------------------------------------------------------------- enfant */

/*
 * Cote enfant : on a ete charge par execve. Tout ce qui suit ne peut reussir
 * que si le descripteur a traverse l'exec.
 */
static int enfant(const char *valeur, const char *ferme)
{
    int fd = atoi(valeur);
    int code = 0;

    struct stat st;
    if (fstat(fd, &st) != 0) {
        fprintf(stderr, "enfant: fstat(%d) : %s\n", fd, strerror(errno));
        code |= 1;
    } else if (!S_ISSOCK(st.st_mode)) {
        fprintf(stderr, "enfant: fd %d n'est pas une socket (mode %o)\n", fd, st.st_mode);
        code |= 2;
    }

    /* Le descripteur marque FD_CLOEXEC ne doit PAS avoir survecu. */
    if (ferme && *ferme) {
        int mort = atoi(ferme);
        if (fcntl(mort, F_GETFD) != -1) {
            fprintf(stderr, "enfant: fd %d marque FD_CLOEXEC a survecu a l'exec\n", mort);
            code |= 4;
        }
    }

    /* Repondre par la socket heritee : c'est la preuve qu'elle est utilisable. */
    const char *reponse = "TAKEOVER-OK";
    if (write(fd, reponse, strlen(reponse)) != (ssize_t)strlen(reponse)) {
        fprintf(stderr, "enfant: write sur fd herite : %s\n", strerror(errno));
        code |= 8;
    }
    return code;
}

/* --------------------------------------------------------------- parent */

int main(int argc, char **argv)
{
    const char *herite = getenv("BOUCHAUD_TAKEOVER_FD");
    if (herite && *herite)
        return enfant(herite, getenv("BOUCHAUD_TAKEOVER_MORT"));

    printf("Sonde heritage de descripteur a travers execve (SOCKET_TAKEOVER)\n\n");

    const char *moi = argc > 0 ? argv[0] : "/bin/exec-fd-probe";

    int paire[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, paire) != 0) {
        printf("  socketpair() impossible : %s\n", strerror(errno));
        printf("EXEC_FD_FAIL echecs=1\n");
        return 1;
    }

    /* L'extremite transmise doit etre explicitement heritable. */
    int drapeaux = fcntl(paire[1], F_GETFD);
    verifie("F_GETFD sur l'extremite a transmettre", drapeaux != -1);
    verifie("F_SETFD peut effacer FD_CLOEXEC", fcntl(paire[1], F_SETFD, drapeaux & ~FD_CLOEXEC) == 0);
    verifie("FD_CLOEXEC bien efface", (fcntl(paire[1], F_GETFD) & FD_CLOEXEC) == 0);

    /* Un temoin qui, lui, doit disparaitre a l'exec. */
    int temoin[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, temoin) != 0) {
        printf("  socketpair() temoin impossible : %s\n", strerror(errno));
        printf("EXEC_FD_FAIL echecs=1\n");
        return 1;
    }
    verifie("F_SETFD peut poser FD_CLOEXEC sur le temoin",
        fcntl(temoin[1], F_SETFD, FD_CLOEXEC) == 0);
    verifie("FD_CLOEXEC bien pose", (fcntl(temoin[1], F_GETFD) & FD_CLOEXEC) != 0);

    /* Le parent doit deja voir une socket, avant tout exec. */
    struct stat st;
    verifie("fstat() sur la socket du parent", fstat(paire[1], &st) == 0);
    verifie("fstat() annonce S_IFSOCK avant exec", S_ISSOCK(st.st_mode));

    char passe[32], mort[32];
    snprintf(passe, sizeof(passe), "%d", paire[1]);
    snprintf(mort, sizeof(mort), "%d", temoin[1]);

    pid_t enfant_pid = fork();
    if (enfant_pid < 0) {
        printf("  fork() impossible : %s\n", strerror(errno));
        printf("EXEC_FD_FAIL echecs=1\n");
        return 1;
    }

    if (enfant_pid == 0) {
        close(paire[0]);
        close(temoin[0]);
        char var[64], var_mort[64];
        snprintf(var, sizeof(var), "BOUCHAUD_TAKEOVER_FD=%s", passe);
        snprintf(var_mort, sizeof(var_mort), "BOUCHAUD_TAKEOVER_MORT=%s", mort);
        char *env[] = { var, var_mort, NULL };
        char *args[] = { (char *)moi, NULL };
        execve(moi, args, env);
        _exit(127);
    }

    close(paire[1]);
    close(temoin[1]);

    char tampon[64];
    ssize_t lus = read(paire[0], tampon, sizeof(tampon) - 1);
    if (lus > 0)
        tampon[lus] = 0;
    else
        tampon[0] = 0;

    int statut = 0;
    waitpid(enfant_pid, &statut, 0);

    verifie("execve() a charge le programme (pas 127)",
        WIFEXITED(statut) && WEXITSTATUS(statut) != 127);
    printf("      code enfant = %d (1=fstat, 2=pas S_IFSOCK, 4=CLOEXEC survit, 8=write)\n",
        WIFEXITED(statut) ? WEXITSTATUS(statut) : -1);
    verifie("le descripteur a survecu a execve et reste une socket",
        WIFEXITED(statut) && (WEXITSTATUS(statut) & 3) == 0);
    verifie("le descripteur FD_CLOEXEC n'a PAS survecu",
        WIFEXITED(statut) && (WEXITSTATUS(statut) & 4) == 0);
    printf("      recu du programme charge : \"%s\"\n", tampon);
    verifie("le programme charge a repondu par la socket heritee",
        strcmp(tampon, "TAKEOVER-OK") == 0);

    printf("\nRESULTAT : %d verification(s) en echec\n", echecs);
    if (echecs == 0)
        printf("EXEC_FD_OK\n");
    else
        printf("EXEC_FD_FAIL echecs=%d\n", echecs);
    return echecs == 0 ? 0 : 1;
}
