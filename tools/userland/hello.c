/*
 * Premier programme libc de Bouchaud OS.
 *
 * Volontairement banal : c'est tout l'interet. S'il s'execute sans
 * modification, c'est que la libc a pu demarrer (auxv, TLS, `set_tid_address`),
 * allouer (`brk`/`mmap`), ecrire (`writev`), lire l'heure et les fichiers, et
 * terminer proprement (`exit_group`) — soit une bonne moitie de l'ABI.
 *
 *   musl-gcc -static-pie -O2 hello.c -o hello
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <sys/utsname.h>

int main(int argc, char **argv, char **envp)
{
    struct utsname u;
    uname(&u);
    printf("Bonjour depuis le ring 3 de %s %s (%s)\n", u.sysname, u.release, u.machine);
    printf("pid=%d uid=%d\n", (int)getpid(), (int)getuid());

    printf("argv:");
    for (int i = 0; i < argc; i++)
        printf(" %s", argv[i]);
    printf("\n");

    int shown = 0;
    for (char **e = envp; *e && shown < 4; e++, shown++)
        printf("env: %s\n", *e);

    /* Tas : passe par brk puis mmap quand la demande grossit. */
    size_t n = 1 << 20;
    char *buffer = malloc(n);
    if (!buffer) {
        fprintf(stderr, "malloc(%zu) a echoue\n", n);
        return 1;
    }
    memset(buffer, 0xA5, n);
    printf("malloc(%zu) OK, controle=%02x\n", n, (unsigned char)buffer[n - 1]);
    free(buffer);

    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    printf("uptime monotone: %ld,%03ld s\n", (long)ts.tv_sec, ts.tv_nsec / 1000000);

    FILE *f = fopen("/tmp-hello.txt", "w+");
    if (f) {
        fputs("ecrit par la libc\n", f);
        rewind(f);
        char line[64] = {0};
        fgets(line, sizeof line, f);
        printf("relecture fichier: %s", line);
        fclose(f);
    }

    puts("termine.");
    return 0;
}
