// Sonde ring 3 : l'exception W^X d'une JVM, et rien qu'elle.
//
// BOUCHAUD_JIT_RWX_V1 -- HotSpot (OpenJDK) ecrit son code compile dans des
// pages qu'il execute ; sans page a la fois inscriptible et executable, la JVM
// ne demarre pas. Bouchaud l'accorde a UN cas : un lanceur Java
// (`/usr/lib/jvm/<jdk>/bin/java`) dont tout le chemin appartient a root et
// n'est inscriptible ni par le groupe ni par les autres, sur de la memoire
// ANONYME et PRIVEE.
//
// La meme sonde est lancee sous plusieurs noms ; le banc dit lequel doit
// recevoir l'exception. Pour chaque voie, elle imprime 1 si la page RWX a ete
// obtenue, 0 si elle a ete refusee :
//
//   anon      mmap(RWX, ANONYMOUS|PRIVATE)  -- et le code ecrit S'EXECUTE
//   partage   mmap(RWX, ANONYMOUS|SHARED)   -- jamais
//   fichier   mmap(RWX, PRIVATE, fichier)   -- jamais
//   mprotect  mprotect(RWX) d'une plage anonyme
//   mpfich    mprotect(RWX) d'une projection de fichier -- jamais
//   trou      mprotect(RWX) d'une plage anonyme trouee -- jamais
//   fils      la voie `anon` dans un fils de fork() : il herite
#define _GNU_SOURCE
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/wait.h>

#define RWX (PROT_READ | PROT_WRITE | PROT_EXEC)
#define PAGE 4096

// mov eax, 42 ; ret
static const unsigned char CODE[] = {0xb8, 0x2a, 0x00, 0x00, 0x00, 0xc3};

static int anon(int *execute) {
    unsigned char *p = mmap(0, PAGE, RWX, MAP_ANONYMOUS | MAP_PRIVATE, -1, 0);
    if (p == MAP_FAILED) return 0;
    memcpy(p, CODE, sizeof CODE);
    int (*f)(void) = (int (*)(void))p;
    int r = f();
    // Reecrire APRES execution, sans mprotect : c'est ce que fait un JIT.
    p[1] = 0x07;
    int r2 = f();
    *execute = r == 42 && r2 == 7;
    munmap(p, PAGE);
    return 1;
}

static int partage(void) {
    void *p = mmap(0, PAGE, RWX, MAP_ANONYMOUS | MAP_SHARED, -1, 0);
    if (p == MAP_FAILED) return 0;
    munmap(p, PAGE);
    return 1;
}

static int fichier(const char *chemin) {
    int fd = open(chemin, O_RDONLY);
    if (fd < 0) return -1;
    void *p = mmap(0, PAGE, RWX, MAP_PRIVATE, fd, 0);
    close(fd);
    if (p == MAP_FAILED) return 0;
    munmap(p, PAGE);
    return 1;
}

static int mprotect_anon(void) {
    void *p = mmap(0, 4 * PAGE, PROT_READ | PROT_WRITE, MAP_ANONYMOUS | MAP_PRIVATE, -1, 0);
    if (p == MAP_FAILED) return -1;
    int ok = mprotect(p, 4 * PAGE, RWX) == 0;
    munmap(p, 4 * PAGE);
    return ok;
}

static int mprotect_fichier(const char *chemin) {
    int fd = open(chemin, O_RDONLY);
    if (fd < 0) return -1;
    void *p = mmap(0, PAGE, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (p == MAP_FAILED) return -1;
    int ok = mprotect(p, PAGE, RWX) == 0;
    munmap(p, PAGE);
    return ok;
}

static int trou(void) {
    unsigned char *p = mmap(0, 3 * PAGE, PROT_READ | PROT_WRITE, MAP_ANONYMOUS | MAP_PRIVATE, -1, 0);
    if (p == MAP_FAILED) return -1;
    munmap(p + PAGE, PAGE);
    int ok = mprotect(p, 3 * PAGE, RWX) == 0;
    munmap(p, PAGE);
    munmap(p + 2 * PAGE, PAGE);
    return ok;
}

static int fils(void) {
    pid_t pid = fork();
    if (pid < 0) return -1;
    if (pid == 0) {
        int execute = 0;
        _exit(anon(&execute) && execute ? 1 : 0);
    }
    int statut = 0;
    if (waitpid(pid, &statut, 0) != pid || !WIFEXITED(statut)) return -1;
    return WEXITSTATUS(statut);
}

int main(int argc, char **argv) {
    const char *role = argc > 1 ? argv[1] : "?";
    // Un fichier ordinaire du disque : la sonde elle-meme.
    const char *cible = argc > 2 ? argv[2] : "/proc/self/exe";
    int execute = 0;
    int a = anon(&execute);
    printf("JIT_RWX_SONDE role=%s anon=%d execute=%d partage=%d fichier=%d mprotect=%d mpfich=%d trou=%d fils=%d\n",
           role, a, execute, partage(), fichier(cible), mprotect_anon(),
           mprotect_fichier(cible), trou(), fils());
    fflush(stdout);
    return 0;
}
