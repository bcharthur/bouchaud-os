/*
 * Sonde : un `fsync` sous /persist pendant que d'autres fils tronquent et
 * suppriment des fichiers persistants.
 *
 * BOUCHAUD_PERSIST_INSTANTANE_UNIQUE_V1
 *
 * La synchronisation de /persist relevait les fichiers (noeud, longueur) sous
 * une prise du RAMFS, la rendait, puis recopiait les contenus sous une
 * SECONDE prise. Un fichier tronque ou supprime entre les deux faisait
 * copier `content[..longueur]` d'un contenu devenu vide : panique noyau
 * (run 37654172489, banc cache et SQL : un service Ladybird ecrit son cache
 * pendant le `fsync` du navigateur ; snapshot.rs:66).
 *
 * Trois fils mutent sans arret /persist/course (creation 32 Kio, troncature
 * a zero, suppression, reecriture courte) ; un quatrieme ecrit un temoin et
 * appelle `fsync` en boucle. Verifie : aucun `fsync` en echec, le temoin
 * relu est le dernier ecrit, et le noyau est vivant a la fin (la ligne de
 * sortie s'imprime). Sortie : PERSIST_COURSE_OK ou PERSIST_COURSE_ECHEC.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define DOSSIER "/persist/course"
#define MUTATEURS 3
#define FICHIERS 12
#define SECONDES 8

static volatile int continuer = 1;
static long mutations[MUTATEURS];

static long maintenant_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

static void *mutateur(void *arg)
{
    int id = (int)(long)arg;
    static char gros[32768];
    memset(gros, 'a' + id, sizeof gros);
    unsigned graine = 12345u + (unsigned)id;
    char chemin[64];
    while (continuer) {
        int k = (int)(rand_r(&graine) % FICHIERS);
        snprintf(chemin, sizeof chemin, DOSSIER "/m%d_%d", id, k);
        switch (rand_r(&graine) % 4) {
        case 0: {
            int fd = open(chemin, O_CREAT | O_WRONLY | O_TRUNC, 0644);
            if (fd >= 0) {
                (void)!write(fd, gros, sizeof gros);
                close(fd);
            }
            break;
        }
        case 1:
            (void)!truncate(chemin, 0);
            break;
        case 2:
            unlink(chemin);
            break;
        default: {
            int fd = open(chemin, O_CREAT | O_WRONLY | O_TRUNC, 0644);
            if (fd >= 0) {
                (void)!write(fd, gros, 100);
                close(fd);
            }
            break;
        }
        }
        mutations[id]++;
    }
    return NULL;
}

int main(void)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    mkdir("/persist", 0755);
    if (mkdir(DOSSIER, 0755) != 0 && errno != EEXIST) {
        printf("PERSIST_COURSE_ECHEC raison=mkdir errno=%d\n", errno);
        return 1;
    }
    pthread_t fils[MUTATEURS];
    for (int i = 0; i < MUTATEURS; i++)
        pthread_create(&fils[i], NULL, mutateur, (void *)(long)i);

    long fsyncs = 0, echecs = 0, dernier = -1;
    long t0 = maintenant_ms();
    while (maintenant_ms() - t0 < SECONDES * 1000) {
        int fd = open(DOSSIER "/temoin", O_CREAT | O_WRONLY | O_TRUNC, 0644);
        if (fd < 0) {
            echecs++;
            continue;
        }
        char ligne[32];
        int n = snprintf(ligne, sizeof ligne, "%ld\n", fsyncs);
        if (write(fd, ligne, (size_t)n) != n)
            echecs++;
        if (fsync(fd) != 0)
            echecs++;
        else
            dernier = fsyncs;
        close(fd);
        fsyncs++;
    }
    continuer = 0;
    long total = 0;
    for (int i = 0; i < MUTATEURS; i++) {
        pthread_join(fils[i], NULL);
        total += mutations[i];
    }

    char relu[32] = { 0 };
    int fd = open(DOSSIER "/temoin", O_RDONLY);
    if (fd >= 0) {
        (void)!read(fd, relu, sizeof relu - 1);
        close(fd);
    }
    int temoin_ok = dernier >= 0 && strtol(relu, NULL, 10) == dernier;
    int ok = echecs == 0 && fsyncs >= 10 && total >= 100 && temoin_ok;
    printf("PERSIST_COURSE fsyncs=%ld mutations=%ld echecs=%ld temoin=%s\n", fsyncs, total, echecs,
        temoin_ok ? "ok" : "ECHEC");
    printf(ok ? "PERSIST_COURSE_OK\n" : "PERSIST_COURSE_ECHEC\n");
    return ok ? 0 : 1;
}
