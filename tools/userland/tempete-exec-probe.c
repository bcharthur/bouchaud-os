/*
 * TEMPETE-EXEC : le motif des gels #283, sans Ladybird (BOUCHAUD_TEMPETE_EXEC_V1).
 *
 *   tempete-exec-probe [SECONDES] [LANCEURS] [MODE]
 *   tempete-exec-probe enfant [MODE]
 *
 * MODE (ce que fait chaque enfant ; `complet` par defaut) :
 *   complet  fil + memoire (fautes, mprotect, mmap/munmap)
 *   mem      memoire seulement, aucun fil
 *   fil      un fil, aucune memoire touchee
 *   vide     execve puis sortie immediate
 *   fork     fork puis _exit, SANS execve
 *
 * Les gels de #283 surviennent pendant la batterie de workers de Ladybird :
 * le navigateur, processus MULTI-FILS, enchaine fork + execve de WebWorker ;
 * chaque nouvel enfant fait aussitot des fautes de page, des mprotect (le
 * chargeur, RELRO), cree des fils ; pendant ce temps `services-metrics` lit
 * le verrou `Mm` de chaque processus. Au gel de main@eb6f115, le coeur 0
 * (`services-metrics`) attendait depuis 820 s le verrou `Mm` d'un WebWorker
 * qui venait d'etre execute.
 *
 * Ce programme rejoue ce motif aussi densement que possible :
 *   - LANCEURS fils, dans un meme processus, font en boucle fork + execve de
 *     ce binaire en mode `enfant`, puis waitpid ;
 *   - un fil fait `poll` sur un tube avec delai (comme une boucle IPC) et un
 *     autre ecrit dans ce tube ;
 *   - l'enfant cree un fil qui touche des pages, et le fil principal touche,
 *     protege (RW -> R -> RW), projette et retire de la memoire.
 *
 * Sortie : `TEMPETE_EXEC t=<s> cycles=<n>` toutes les 5 s, puis
 * `TEMPETE_EXEC_FIN cycles=<n> echecs=<n>`. Un noyau fige ne l'ecrit jamais.
 */
#include <errno.h>
#include <poll.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define CHEMIN "/bin/tempete-exec-probe"
#define PAGE 4096
#define ENFANT_PAGES 64

static atomic_long cycles;
static atomic_long echecs;
static atomic_int fin;
static const char *mode_enfant = "complet";
static int tube[2];

static double maintenant(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

/* ---- mode enfant -------------------------------------------------------- */

static void *fil_toucheur(void *arg) {
    volatile char *p = arg;
    for (int i = 0; i < ENFANT_PAGES; i++) {
        p[i * PAGE] = (char)i;
    }
    return NULL;
}

static int enfant(const char *mode) {
    if (strcmp(mode, "vide") == 0) {
        return 0;
    }
    if (strcmp(mode, "fil") == 0) {
        pthread_t t;
        static char rien[PAGE * ENFANT_PAGES];
        if (pthread_create(&t, NULL, fil_toucheur, rien) != 0) return 5;
        pthread_join(t, NULL);
        return 0;
    }
    int avec_fil = strcmp(mode, "mem") != 0;
    size_t taille = ENFANT_PAGES * PAGE;
    char *a = mmap(NULL, taille, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *b = mmap(NULL, taille, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (a == MAP_FAILED || b == MAP_FAILED) {
        return 2;
    }
    pthread_t t;
    int fil = avec_fil && pthread_create(&t, NULL, fil_toucheur, b) == 0;
    if (!avec_fil) fil_toucheur(b);
    for (int tour = 0; tour < 8; tour++) {
        for (int i = 0; i < ENFANT_PAGES; i += 3) {
            a[i * PAGE] = (char)tour;
        }
        if (mprotect(a, taille, PROT_READ) != 0) {
            return 3;
        }
        volatile char lu = 0;
        for (int i = 0; i < ENFANT_PAGES; i += 5) {
            lu ^= a[i * PAGE];
        }
        (void)lu;
        if (mprotect(a, taille, PROT_READ | PROT_WRITE) != 0) {
            return 4;
        }
        char *c = mmap(NULL, 4 * PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (c != MAP_FAILED) {
            c[0] = 1;
            c[3 * PAGE] = 2;
            munmap(c, 4 * PAGE);
        }
    }
    if (fil) {
        pthread_join(t, NULL);
    }
    return 0;
}

/* ---- mode parent -------------------------------------------------------- */

static void *lanceur(void *arg) {
    (void)arg;
    while (!atomic_load(&fin)) {
        pid_t p = fork();
        if (p == 0 && strcmp(mode_enfant, "fork") == 0) {
            _exit(0);
        }
        if (p == 0) {
            char *const argv[] = {CHEMIN, "enfant", (char *)mode_enfant, NULL};
            char *const envp[] = {NULL};
            execve(CHEMIN, argv, envp);
            _exit(127);
        }
        if (p < 0) {
            atomic_fetch_add(&echecs, 1);
            continue;
        }
        int statut = 0;
        if (waitpid(p, &statut, 0) != p || !WIFEXITED(statut) || WEXITSTATUS(statut) != 0) {
            atomic_fetch_add(&echecs, 1);
        }
        atomic_fetch_add(&cycles, 1);
    }
    return NULL;
}

static void *ecouteur(void *arg) {
    (void)arg;
    char tampon[64];
    while (!atomic_load(&fin)) {
        struct pollfd pfd = {.fd = tube[0], .events = POLLIN};
        if (poll(&pfd, 1, 50) > 0) {
            if (read(tube[0], tampon, sizeof tampon) < 0 && errno != EINTR) {
                break;
            }
        }
    }
    return NULL;
}

static void *ecrivain(void *arg) {
    (void)arg;
    while (!atomic_load(&fin)) {
        if (write(tube[1], "x", 1) < 0 && errno != EINTR) {
            break;
        }
        usleep(2000);
    }
    return NULL;
}

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "enfant") == 0) {
        return enfant(argc > 2 ? argv[2] : "complet");
    }
    int secondes = argc > 1 ? atoi(argv[1]) : 60;
    int lanceurs = argc > 2 ? atoi(argv[2]) : 3;
    if (secondes <= 0) secondes = 60;
    if (lanceurs <= 0 || lanceurs > 16) lanceurs = 3;
    if (argc > 3) mode_enfant = argv[3];
    if (pipe(tube) != 0) {
        printf("TEMPETE_EXEC_ECHEC pipe errno=%d\n", errno);
        return 1;
    }
    printf("TEMPETE_EXEC_DEBUT secondes=%d lanceurs=%d mode=%s\n", secondes, lanceurs, mode_enfant);
    fflush(stdout);
    pthread_t l[16], e, w;
    pthread_create(&e, NULL, ecouteur, NULL);
    pthread_create(&w, NULL, ecrivain, NULL);
    for (int i = 0; i < lanceurs; i++) {
        pthread_create(&l[i], NULL, lanceur, NULL);
    }
    double debut = maintenant(), prochain = debut + 5;
    while (maintenant() - debut < secondes) {
        usleep(200000);
        if (maintenant() >= prochain) {
            printf("TEMPETE_EXEC t=%d cycles=%ld\n", (int)(maintenant() - debut), atomic_load(&cycles));
            fflush(stdout);
            prochain += 5;
        }
    }
    atomic_store(&fin, 1);
    for (int i = 0; i < lanceurs; i++) {
        pthread_join(l[i], NULL);
    }
    pthread_join(e, NULL);
    pthread_join(w, NULL);
    printf("TEMPETE_EXEC_FIN mode=%s cycles=%ld echecs=%ld\n", mode_enfant, atomic_load(&cycles), atomic_load(&echecs));
    return atomic_load(&echecs) == 0 ? 0 : 1;
}
