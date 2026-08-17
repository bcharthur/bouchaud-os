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

// Keep Ladybird's control socket far away from 0/1/2 and from the surface/GUI
// descriptors installed by Bouchaud's window manager (normally the first free
// descriptors after stdio). M7 used fd 3 because it was launched directly from
// the shell; doing that inside a real GUI client would overwrite BO_SURFACE_FD.
#define WEBCONTENT_CONTROL_FD 100

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

int main(int argc, char** argv) {
    char const* webcontent = argc > 1 ? argv[1] : "/usr/libexec/ladybird/WebContent";
    int fd[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, fd) < 0) return fail("socketpair");

    pid_t pid = fork();
    if (pid < 0) return fail("fork");
    if (pid == 0) {
        close(fd[0]);
        if (dup2(fd[1], WEBCONTENT_CONTROL_FD) < 0) _exit(120);
        if (fd[1] != WEBCONTENT_CONTROL_FD) close(fd[1]);
        char control_fd[16];
        snprintf(control_fd, sizeof(control_fd), "%d", WEBCONTENT_CONTROL_FD);
        setenv("BOUCHAUD_WEBCONTENT_FD", control_fd, 1);
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

    // M8 has a finite contract: WebContent parses the local document, renders a
    // CPU screenshot, copies it to the Bouchaud window surface, announces a
    // FrameReady, and exits 0. The parent deliberately stays alive two more
    // seconds so the WM can consume the frame before the GUI client disappears.
    if (getenv("BOUCHAUD_M8")) {
        for (int i = 0; i < 150; ++i) {
            sleep(1);
            drain_peer(fd[0]);
            int status = 0;
            pid_t r = waitpid(pid, &status, WNOHANG);
            if (r == pid) {
                if (WIFEXITED(status) && WEXITSTATUS(status) == 0) {
                    printf("[ladybird-bouchaud] M8_WEBCONTENT_EXIT_OK\n");
                    printf("[ladybird-bouchaud] RESULTAT : M8 HTML local dans fenetre Bouchaud OK\n");
                    fflush(stdout);
                    sleep(2);
                    close(fd[0]);
                    return 0;
                }
                if (WIFEXITED(status))
                    printf("[ladybird-bouchaud] ECHEC M8 WebContent termine code=%d\n", WEXITSTATUS(status));
                else if (WIFSIGNALED(status))
                    printf("[ladybird-bouchaud] ECHEC M8 WebContent signal=%d\n", WTERMSIG(status));
                fflush(stdout);
                close(fd[0]);
                return 2;
            }
        }
        printf("[ladybird-bouchaud] ECHEC M8 timeout WebContent\n");
        fflush(stdout);
        kill(pid, SIGTERM);
        sleep(1);
        kill(pid, SIGKILL);
        waitpid(pid, NULL, 0);
        close(fd[0]);
        return 3;
    }

    // M7 contract: give the complete LibWeb/JS/Gfx initialization a few
    // seconds. If the process is still alive, exec + static loader + initial
    // initialization succeeded.
    for (int i = 0; i < 6; ++i) {
        sleep(1);
        drain_peer(fd[0]);
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
