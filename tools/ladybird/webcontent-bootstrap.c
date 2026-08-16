// Bouchaud M7/M8 bootstrap: launch the real Ladybird WebContent as a separate
// process with an inherited AF_LOCAL socket. No fake IPC protocol is involved;
// WebContent itself adopts this fd through the tiny Bouchaud upstream adapter.
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

static int fail(char const* what) {
    fprintf(stderr, "[ladybird-bouchaud] ECHEC %s: %s\n", what, strerror(errno));
    return 1;
}

int main(int argc, char** argv) {
    char const* webcontent = argc > 1 ? argv[1] : "/usr/libexec/ladybird/WebContent";
    int fd[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, fd) < 0) return fail("socketpair");

    pid_t pid = fork();
    if (pid < 0) return fail("fork");
    if (pid == 0) {
        close(fd[0]);
        if (dup2(fd[1], 3) < 0) _exit(120);
        if (fd[1] != 3) close(fd[1]);
        setenv("BOUCHAUD_WEBCONTENT_FD", "3", 1);
        char* const args[] = {
            (char*)webcontent,
            (char*)"--disable-sandbox",
            (char*)"--headless",
            (char*)"--force-fontconfig",
            (char*)"--site-isolation", (char*)"disable",
            (char*)"--config-path", (char*)"/usr/share/ladybird",
            NULL
        };
        execv(webcontent, args);
        perror("exec WebContent");
        _exit(121);
    }

    close(fd[1]);
    printf("[ladybird-bouchaud] WebContent lance pid=%d\n", (int)pid);
    fflush(stdout);

    // Give the complete LibWeb/JS/Gfx initialization a few seconds. If the
    // process is still alive, exec + static loader + initialisation succeeded.
    for (int i = 0; i < 6; ++i) {
        sleep(1);
        int status = 0;
        pid_t r = waitpid(pid, &status, WNOHANG);
        if (r == pid) {
            if (WIFEXITED(status))
                printf("[ladybird-bouchaud] ECHEC WebContent termine code=%d\n", WEXITSTATUS(status));
            else if (WIFSIGNALED(status))
                printf("[ladybird-bouchaud] ECHEC WebContent signal=%d\n", WTERMSIG(status));
            return 2;
        }
    }

    printf("[ladybird-bouchaud] WEBCONTENT_PROCESS_ALIVE\n");
    printf("[ladybird-bouchaud] RESULTAT : WebContent natif ring3 OK\n");
    fflush(stdout);
    kill(pid, SIGTERM);
    sleep(1);
    kill(pid, SIGKILL);
    waitpid(pid, NULL, 0);
    close(fd[0]);
    return 0;
}
