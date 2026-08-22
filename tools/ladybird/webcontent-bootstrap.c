// Lanceur des services Ladybird sous Bouchaud OS.
//
// Il tient la place que le processus UI occupe chez upstream : creer les paires
// de sockets, demarrer chaque service, et donner a WebContent le descripteur de
// chacun. Les services sont les binaires upstream, non modifies pour deux
// d'entre eux (ImageDecoder par `SOCKET_TAKEOVER`, exactement comme
// `LibWebView/Process.cpp`).
//
// Services demarres :
//   ImageDecoder   decodage PNG/JPEG/WebP/GIF/... hors du moteur
//   RequestServer  DNS, TCP, TLS, HTTP           (chemin reseau)
//   WebContent     le moteur lui-meme
//
// Le processus lance par le gestionnaire de fenetres reste ce lanceur : fermer
// la fenetre Bouchaud termine donc tout l'arbre de descendants.
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define WEBCONTENT_CONTROL_FD 100
#define REQUESTSERVER_FD 101
// Cote service : le descripteur que `SOCKET_TAKEOVER` nomme a ImageDecoder.
#define IMAGEDECODER_SERVICE_FD 102
// Cote moteur : le descripteur que WebContent adopte pour parler au decodeur.
#define IMAGEDECODER_CLIENT_FD 103

// Fontconfig cherche sa configuration dans `FONTCONFIG_FILE`, puis dans
// `$FONTCONFIG_PATH/fonts.conf`, puis dans un chemin fige a la compilation de
// la bibliotheque — celui de la machine Ubuntu qui a construit le paquet, et
// qui n'existe pas ici. D'ou, au demarrage :
//
//     Fontconfig error: Cannot load default config file: No such file: (null)
//
// Ce n'est pas un avertissement decoratif : `SkFontMgr_New_FontConfig` est le
// gestionnaire de polices de Skia, donc tout le repli quand une page demande
// une famille que le fournisseur de chemins de Ladybird ne connait pas. Sans
// configuration, ce gestionnaire ne voit aucune police.
//
// Les deux variables sont posees dans le lanceur, avant tout `fork`, pour que
// chaque service en herite : WebContent rasterise, ImageDecoder lie LibGfx.
static void designer_fontconfig(void) {
    static char const* const config = "/usr/share/ladybird/fontconfig/fonts.conf";
    if (access(config, R_OK) != 0) {
        printf("[ladybird-bouchaud] FONTCONFIG_ABSENT %s\n", config);
        fflush(stdout);
        return;
    }
    setenv("FONTCONFIG_FILE", config, 1);
    setenv("FONTCONFIG_PATH", "/usr/share/ladybird/fontconfig", 1);
    // Sans repertoire de cache inscriptible, fontconfig reanalyse chaque
    // fichier de police a chaque appel. /tmp est le seul emplacement
    // inscriptible d'un systeme de fichiers reconstruit a chaque demarrage.
    mkdir("/tmp/fontconfig", 0777);
    printf("[ladybird-bouchaud] FONTCONFIG %s\n", config);
    fflush(stdout);
}

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

// ImageDecoder, lance exactement comme le lanceur d'upstream le fait
// (`LibWebView/Process.cpp`) : une paire de sockets, `SOCKET_TAKEOVER` qui
// nomme l'extremite du service, et le binaire upstream **non modifie** qui
// l'adopte par `IPC::take_over_accepted_client_from_system_server`.
//
// Le service refuse le descripteur si `fstat` ne le declare pas `S_ISSOCK` :
// c'est une verification d'upstream, et c'est elle qui a fait remonter le
// defaut correspondant dans `sys_fstat` du noyau.
static pid_t launch_image_decoder(char const* path, int service_fd, int other_fd) {
    pid_t pid = fork();
    if (pid != 0)
        return pid;

    if (dup2(service_fd, IMAGEDECODER_SERVICE_FD) < 0)
        _exit(140);

    if (service_fd != IMAGEDECODER_SERVICE_FD)
        close(service_fd);
    if (other_fd >= 0 && other_fd != IMAGEDECODER_SERVICE_FD)
        close(other_fd);

    char takeover[48];
    snprintf(takeover, sizeof(takeover), "ImageDecoder:%d", IMAGEDECODER_SERVICE_FD);
    setenv("SOCKET_TAKEOVER", takeover, 1);

    char* const args[] = {
        (char*)path,
        (char*)"--disable-sandbox",
        NULL
    };

    execv(path, args);
    perror("exec ImageDecoder");
    _exit(141);
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

// Demarre le service de decodage et rend le descripteur destine au moteur.
//
// Rend -1 si le binaire n'est pas installe. Ce cas n'est pas silencieux : le
// journal le dit ici, WebContent le redit au demarrage, et la premiere image
// rencontree fera tomber `VERIFY(s_the)` — c'est-a-dire que l'absence se voit
// avant de couter une execution complete.
static pid_t start_image_decoder(char const* path, int* client_fd) {
    *client_fd = -1;

    if (access(path, X_OK) != 0) {
        printf("[ladybird-bouchaud] IMAGE_DECODER_INTROUVABLE %s\n", path);
        fflush(stdout);
        return -1;
    }

    int pair[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, pair) < 0) {
        fail("socketpair ImageDecoder");
        return -1;
    }

    pid_t pid = launch_image_decoder(path, pair[0], pair[1]);
    if (pid < 0) {
        close(pair[0]);
        close(pair[1]);
        fail("fork ImageDecoder");
        return -1;
    }

    close(pair[0]);
    *client_fd = pair[1];
    printf("[ladybird-bouchaud] IMAGE_DECODER_LANCE pid=%d fd=%d\n", (int)pid, pair[1]);
    fflush(stdout);
    return pid;
}

static pid_t launch_webcontent(
    char const* path,
    int control_child,
    int control_parent,
    int request_child,
    int request_parent,
    int image_child,
    int image_parent,
    int m9)
{
    pid_t pid = fork();
    if (pid != 0)
        return pid;

    if (dup2(control_child, WEBCONTENT_CONTROL_FD) < 0)
        _exit(120);

    if (m9 && dup2(request_child, REQUESTSERVER_FD) < 0)
        _exit(122);

    // Le decodeur d'images ne depend d'aucun jalon : il sert la page locale
    // comme le site distant. Son descripteur suit donc sa propre disponibilite,
    // pas `m9`.
    if (image_child >= 0 && dup2(image_child, IMAGEDECODER_CLIENT_FD) < 0)
        _exit(123);

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

    if (image_child >= 0) {
        if (image_child != IMAGEDECODER_CLIENT_FD)
            close(image_child);
        if (image_parent >= 0 && image_parent != IMAGEDECODER_CLIENT_FD)
            close(image_parent);
    }

    char control_fd[16];
    snprintf(control_fd, sizeof(control_fd), "%d", WEBCONTENT_CONTROL_FD);
    setenv("BOUCHAUD_WEBCONTENT_FD", control_fd, 1);

    if (m9) {
        char request_fd[16];
        snprintf(request_fd, sizeof(request_fd), "%d", REQUESTSERVER_FD);
        setenv("BOUCHAUD_REQUEST_FD", request_fd, 1);
    }

    if (image_child >= 0) {
        char image_fd[16];
        snprintf(image_fd, sizeof(image_fd), "%d", IMAGEDECODER_CLIENT_FD);
        setenv("BOUCHAUD_IMAGEDECODER_FD", image_fd, 1);
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

static int run_m8(char const* webcontent, char const* imagedecoder) {
    // Le decodeur part en premier : il n'herite ainsi d'aucun descripteur des
    // autres canaux, et la fermeture de la fenetre continue de propager son EOF
    // a qui de droit.
    int image_fd = -1;
    pid_t image_pid = start_image_decoder(imagedecoder, &image_fd);

    int control[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, control) < 0)
        return fail("socketpair M8 control");

    pid_t web_pid = launch_webcontent(
        webcontent, control[1], control[0], -1, -1, image_fd, -1, 0);
    if (web_pid < 0) {
        terminate_child(image_pid);
        return fail("fork WebContent");
    }

    close(control[1]);
    if (image_fd >= 0)
        close(image_fd);

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
            terminate_child(image_pid);
            close(control[0]);
            return 0;
        }

        if (WIFEXITED(status))
            printf("[ladybird-bouchaud] ECHEC M8 WebContent termine code=%d\n", WEXITSTATUS(status));
        else if (WIFSIGNALED(status))
            printf("[ladybird-bouchaud] ECHEC M8 WebContent signal=%d\n", WTERMSIG(status));
        fflush(stdout);
        terminate_child(image_pid);
        close(control[0]);
        return 2;
    }

    printf("[ladybird-bouchaud] ECHEC M8 timeout WebContent\n");
    fflush(stdout);
    terminate_child(web_pid);
    terminate_child(image_pid);
    close(control[0]);
    return 3;
}

static int run_m9(char const* webcontent, char const* requestserver, char const* imagedecoder) {
    int control[2];
    int request[2];

    // Meme raison qu'en M8 : le decodeur d'abord, avant que les autres paires
    // n'existent, pour qu'il n'en herite aucune extremite.
    int image_fd = -1;
    pid_t image_pid = start_image_decoder(imagedecoder, &image_fd);

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
        webcontent, control[1], control[0], request[1], request[0], image_fd, -1, 1);
    if (web_pid < 0) {
        terminate_child(request_pid);
        terminate_child(image_pid);
        return fail("fork WebContent");
    }

    close(control[1]);
    close(request[0]);
    close(request[1]);
    if (image_fd >= 0)
        close(image_fd);

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
            terminate_child(image_pid);
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
            terminate_child(image_pid);
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
        terminate_child(image_pid);
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
    char const* imagedecoder = argc > 3
        ? argv[3]
        : "/usr/libexec/ladybird/ImageDecoder";

    designer_fontconfig();

    if (getenv("BOUCHAUD_M8"))
        return run_m8(webcontent, imagedecoder);

    if (getenv("BOUCHAUD_M9"))
        return run_m9(webcontent, requestserver, imagedecoder);

    // Historical M7 liveness check.
    int control[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, control) < 0)
        return fail("socketpair");
    pid_t pid = launch_webcontent(webcontent, control[1], control[0], -1, -1, -1, -1, 0);
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
