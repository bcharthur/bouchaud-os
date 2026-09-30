/*
 * tcp-connexions-probe : des centaines de connexions TCP successives vers le
 * meme serveur, pour voir si l'une d'elles reste sans reponse.
 *
 * POURQUOI CETTE SONDE
 *
 * Smoke Ladybird #373 : `fetch-texte` echoue apres 25,6 s (« Could not
 * connect »), cinq SYN sans reponse, alors que la connexion suivante vers le
 * MEME serveur aboutit en 140 ms. `TcpConn::connect` tire son port source
 * dans 4 096 valeurs (`0xC000 | rdtsc & 0x0FFF`), sans tenir compte des ports
 * deja utilises, et garde le meme pour ses cinq essais. Une collision avec une
 * connexion que SLIRP garde encore suffirait a expliquer les cinq echecs.
 * C'est une hypothese : cette sonde la met a l'epreuve.
 *
 *     tcp-connexions-probe IP PORT [CONNEXIONS]
 *
 * Chaque tour : socket, connect, requete HTTP/1.0, lecture jusqu'a la fin,
 * close. Verdict : `TCP_CONNEXIONS_OK n=N pire_ms=P` si toutes aboutissent,
 * sinon `TCP_CONNEXIONS_FAIL echecs=E n=N pire_ms=P` et code 1.
 */
#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

static long ms_maintenant(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000L + t.tv_nsec / 1000000L;
}

int main(int argc, char **argv)
{
    if (argc < 3) {
        printf("TCP_CONNEXIONS_FAIL usage: tcp-connexions-probe IP PORT [CONNEXIONS]\n");
        return 1;
    }
    int n = argc > 3 ? atoi(argv[3]) : 200;
    struct sockaddr_in cible;
    memset(&cible, 0, sizeof(cible));
    cible.sin_family = AF_INET;
    cible.sin_port = htons((unsigned short)atoi(argv[2]));
    if (inet_pton(AF_INET, argv[1], &cible.sin_addr) != 1) {
        printf("TCP_CONNEXIONS_FAIL adresse\n");
        return 1;
    }
    int echecs = 0;
    long pire = 0;
    static const char requete[] = "GET / HTTP/1.0\r\n\r\n";
    char tampon[1024];
    for (int i = 0; i < n; i++) {
        int s = socket(AF_INET, SOCK_STREAM, 0);
        if (s < 0) {
            echecs++;
            continue;
        }
        long debut = ms_maintenant();
        int r = connect(s, (struct sockaddr *)&cible, sizeof(cible));
        long duree = ms_maintenant() - debut;
        if (duree > pire)
            pire = duree;
        if (r != 0) {
            echecs++;
            printf("TCP_CONNEXIONS_ECHEC tour=%d ms=%ld\n", i, duree);
            close(s);
            continue;
        }
        if (write(s, requete, sizeof(requete) - 1) < 0) {
            echecs++;
            close(s);
            continue;
        }
        while (read(s, tampon, sizeof(tampon)) > 0)
            ;
        close(s);
        if ((i + 1) % 50 == 0)
            printf("TCP_CONNEXIONS_PROGRES n=%d echecs=%d pire_ms=%ld\n", i + 1, echecs, pire);
    }
    if (echecs != 0) {
        printf("TCP_CONNEXIONS_FAIL echecs=%d n=%d pire_ms=%ld\n", echecs, n, pire);
        return 1;
    }
    printf("TCP_CONNEXIONS_OK n=%d pire_ms=%ld\n", n, pire);
    return 0;
}
