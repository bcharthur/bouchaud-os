// Bouchaud M7/M8/M9 bootstrap.
//
// M8: WebContent only, finite local-HTML regression test.
// M9: RequestServer + WebContent connected with a real Ladybird IPC socket.
//     RequestServer performs HTTP through Bouchaud's POSIX socket ABI.
//
// The process launched by the WM remains this bootstrap, so closing the Bouchaud
// window terminates the whole descendant tree (RequestServer + WebContent).
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

#define WEBCONTENT_CONTROL_FD 100
#define REQUESTSERVER_FD 101

static int fail(char const* what) {
    fprintf(stderr, "[ladybird-bouchaud] ECHEC %s: %s\n", what, strerror(errno));
    return 1;
}

static void drain_peer(int fd) {
    char buffer[4096];
    for (;;) {
        ssize_t n = recv(fd, buffer, sizeof(buffer), MSG_DONTWAIT);
        if (n > 0) continue;
        if (n < 0 && errno == EINTR) continue;
        break;
    }
}

static void terminate_child(pid_t pid) {
    if (pid <= 0) return;
    kill(pid, SIGTERM);
    sleep(1);
    kill(pid, SIGKILL);
    waitpid(pid, NULL, 0);
}

static pid_t launch_request_server(char const* path, int server_fd, int other_fd) {
    pid_t pid = fork();
    if (pid != 0)
        return pid;

    if (dup2(server_fd, REQUESTSERVER_FD) < 0)
        _exit(130);

    if (server_fd != REQUESTSERVER_FD)
        close(server_fd);
    if (other_fd >= 0 && other_fd != REQUESTSERVER_FD)
        close(other_fd);

    char inherited[16];
    snprintf(inherited, sizeof(inherited), "%d", REQUESTSERVER_FD);
    setenv("BOUCHAUD_REQUESTSERVER_FD", inherited, 1);

    // Certificats racine. RequestServer pose le chemin recu en `CURLOPT_CAINFO` ;
    // sans lui, curl retombe sur le chemin compile a la construction, qui
    // n'existe pas sous Bouchaud — et toute connexion TLS echoue a la
    // verification, meme contre un serveur parfaitement valide.
    //
    // Le paquet n'est ajoute que si le fichier est **lisible**. C'est ce qui
    // permet a M9 (HTTP simple, sans certificats embarques) de continuer a
    // fonctionner exactement comme avant : pas de fichier, pas d'argument.
    char const* ca = getenv("BOUCHAUD_CA_BUNDLE");
    if (ca == NULL || *ca == '\0')
        ca = "/etc/ssl/certs/ca-certificates.crt";
    int have_ca = access(ca, R_OK) == 0;
    if (have_ca)
        printf("[ladybird-bouchaud] M12_CA_BUNDLE %s\n", ca);
    else
        printf("[ladybird-bouchaud] M12_CA_BUNDLE absent (%s) — TLS indisponible\n", ca);
    fflush(stdout);

    char* const args_tls[] = {
        (char*)path,
        (char*)"--disable-sandbox",
        (char*)"--http-disk-cache-mode", (char*)"disabled",
        (char*)"--cache-path", (char*)"/tmp/ladybird-cache",
        (char*)"--certificate", (char*)ca,
        NULL
    };
    char* const args[] = {
        (char*)path,
        (char*)"--disable-sandbox",
        (char*)"--http-disk-cache-mode", (char*)"disabled",
        (char*)"--cache-path", (char*)"/tmp/ladybird-cache",
        NULL
    };

    execv(path, have_ca ? args_tls : args);
    perror("exec RequestServer");
    _exit(131);
}

static pid_t launch_webcontent(
    char const* path,
    int control_child,
    int control_parent,
    int request_child,
    int request_parent,
    int m9)
{
    pid_t pid = fork();
    if (pid != 0)
        return pid;

    if (dup2(control_child, WEBCONTENT_CONTROL_FD) < 0)
        _exit(120);

    if (m9 && dup2(request_child, REQUESTSERVER_FD) < 0)
        _exit(122);

    if (control_child != WEBCONTENT_CONTROL_FD)
        close(control_child);
    if (control_parent >= 0 && control_parent != WEBCONTENT_CONTROL_FD)
        close(control_parent);

    if (m9) {
        if (request_child != REQUESTSERVER_FD)
            close(request_child);
        if (request_parent >= 0 && request_parent != REQUESTSERVER_FD)
            close(request_parent);
    }

    char control_fd[16];
    snprintf(control_fd, sizeof(control_fd), "%d", WEBCONTENT_CONTROL_FD);
    setenv("BOUCHAUD_WEBCONTENT_FD", control_fd, 1);

    if (m9) {
        char request_fd[16];
        snprintf(request_fd, sizeof(request_fd), "%d", REQUESTSERVER_FD);
        setenv("BOUCHAUD_REQUEST_FD", request_fd, 1);
    }

    char* const args[] = {
        (char*)path,
        (char*)"--disable-sandbox",
        (char*)"--headless",
        (char*)"--force-fontconfig",
        (char*)"--site-isolation", (char*)"disable",
        (char*)"--config-path", (char*)"/usr/share/ladybird",
        NULL
    };

    execv(path, args);
    perror("exec WebContent");
    _exit(121);
}

static int run_m8(char const* webcontent) {
    int control[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, control) < 0)
        return fail("socketpair M8 control");

    pid_t web_pid = launch_webcontent(
        webcontent, control[1], control[0], -1, -1, 0);
    if (web_pid < 0)
        return fail("fork WebContent");

    close(control[1]);

    printf("[ladybird-bouchaud] WebContent lance pid=%d\n", (int)web_pid);
    fflush(stdout);

    for (int i = 0; i < 150; ++i) {
        sleep(1);
        drain_peer(control[0]);

        int status = 0;
        pid_t r = waitpid(web_pid, &status, WNOHANG);
        if (r != web_pid)
            continue;

        if (WIFEXITED(status) && WEXITSTATUS(status) == 0) {
            printf("[ladybird-bouchaud] M8_WEBCONTENT_EXIT_OK\n");
            printf("[ladybird-bouchaud] RESULTAT : M8 HTML local dans fenetre Bouchaud OK\n");
            fflush(stdout);
            sleep(2);
            close(control[0]);
            return 0;
        }

        if (WIFEXITED(status))
            printf("[ladybird-bouchaud] ECHEC M8 WebContent termine code=%d\n", WEXITSTATUS(status));
        else if (WIFSIGNALED(status))
            printf("[ladybird-bouchaud] ECHEC M8 WebContent signal=%d\n", WTERMSIG(status));
        fflush(stdout);
        close(control[0]);
        return 2;
    }

    printf("[ladybird-bouchaud] ECHEC M8 timeout WebContent\n");
    fflush(stdout);
    terminate_child(web_pid);
    close(control[0]);
    return 3;
}

static int run_m9(char const* webcontent, char const* requestserver) {
    int control[2];
    int request[2];

    if (socketpair(AF_UNIX, SOCK_STREAM, 0, control) < 0)
        return fail("socketpair M9 control");
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, request) < 0)
        return fail("socketpair M9 RequestServer");

    pid_t request_pid = launch_request_server(requestserver, request[0], request[1]);
    if (request_pid < 0)
        return fail("fork RequestServer");

    printf("[ladybird-bouchaud] M9_REQUESTSERVER_LAUNCHED pid=%d\n", (int)request_pid);
    fflush(stdout);

    pid_t web_pid = launch_webcontent(
        webcontent, control[1], control[0], request[1], request[0], 1);
    if (web_pid < 0) {
        terminate_child(request_pid);
        return fail("fork WebContent");
    }

    close(control[1]);
    close(request[0]);
    close(request[1]);

    printf("[ladybird-bouchaud] WebContent lance pid=%d\n", (int)web_pid);
    fflush(stdout);

    int finite_test = getenv("BOUCHAUD_M9_TEST") != NULL;

    // In M9 test mode WebContent exits 0 immediately after the verified frame.
    // In interactive M9 it remains alive until the WM closes the bootstrap tree.
    for (;;) {
        sleep(1);
        drain_peer(control[0]);

        int request_status = 0;
        pid_t rr = waitpid(request_pid, &request_status, WNOHANG);
        if (rr == request_pid) {
            printf("[ladybird-bouchaud] ECHEC M9 RequestServer termine prematurement");
            if (WIFEXITED(request_status))
                printf(" code=%d", WEXITSTATUS(request_status));
            else if (WIFSIGNALED(request_status))
                printf(" signal=%d", WTERMSIG(request_status));
            printf("\n");
            fflush(stdout);
            terminate_child(web_pid);
            close(control[0]);
            return 4;
        }

        int web_status = 0;
        pid_t wr = waitpid(web_pid, &web_status, WNOHANG);
        if (wr != web_pid)
            continue;

        if (finite_test && WIFEXITED(web_status) && WEXITSTATUS(web_status) == 0) {
            printf("[ladybird-bouchaud] M9_WEBCONTENT_EXIT_OK\n");
            printf("[ladybird-bouchaud] RESULTAT : M9 HTTP distant dans fenetre Bouchaud OK\n");
            fflush(stdout);

            terminate_child(request_pid);
            sleep(2);
            close(control[0]);
            return 0;
        }

        if (WIFEXITED(web_status))
            printf("[ladybird-bouchaud] ECHEC M9 WebContent termine code=%d\n", WEXITSTATUS(web_status));
        else if (WIFSIGNALED(web_status))
            printf("[ladybird-bouchaud] ECHEC M9 WebContent signal=%d\n", WTERMSIG(web_status));
        fflush(stdout);

        terminate_child(request_pid);
        close(control[0]);
        return 5;
    }
}

int main(int argc, char** argv) {
    char const* webcontent = argc > 1
        ? argv[1]
        : "/usr/libexec/ladybird/WebContent";
    char const* requestserver = argc > 2
        ? argv[2]
        : "/usr/libexec/ladybird/RequestServer";

    if (getenv("BOUCHAUD_M8"))
        return run_m8(webcontent);

    if (getenv("BOUCHAUD_M9"))
        return run_m9(webcontent, requestserver);

    // Historical M7 liveness check.
    int control[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, control) < 0)
        return fail("socketpair");
    pid_t pid = launch_webcontent(webcontent, control[1], control[0], -1, -1, 0);
    if (pid < 0)
        return fail("fork");

    close(control[1]);
    printf("[ladybird-bouchaud] WebContent lance pid=%d\n", (int)pid);
    fflush(stdout);

    for (int i = 0; i < 6; ++i) {
        sleep(1);
        drain_peer(control[0]);
        int status = 0;
        pid_t r = waitpid(pid, &status, WNOHANG);
        if (r == pid) {
            if (WIFEXITED(status))
                printf("[ladybird-bouchaud] ECHEC WebContent termine code=%d\n", WEXITSTATUS(status));
            else if (WIFSIGNALED(status))
                printf("[ladybird-bouchaud] ECHEC WebContent signal=%d\n", WTERMSIG(status));
            close(control[0]);
            return 2;
        }
    }

    printf("[ladybird-bouchaud] WEBCONTENT_PROCESS_ALIVE\n");
    printf("[ladybird-bouchaud] RESULTAT : WebContent natif ring3 OK\n");
    fflush(stdout);
    terminate_child(pid);
    close(control[0]);
    return 0;
}
