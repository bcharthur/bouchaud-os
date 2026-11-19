#include "bouchaud-gpu.h"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <signal.h>
#include <sys/wait.h>
#include <stdlib.h>
#include <unistd.h>

static uint32_t pixel(unsigned x, unsigned y, unsigned motif)
{
    return ((x * 31337) ^ (y * 12345) ^ motif) & 0xffffff;
}

struct canaux { int lecture, ecriture; };

static int echange(const struct canaux *canaux, uint32_t valeur, uint32_t *distant)
{
    if (!canaux) return 0;
    if (write(canaux->ecriture, &valeur, sizeof(valeur)) != sizeof(valeur)
        || read(canaux->lecture, distant, sizeof(*distant)) != sizeof(*distant))
        return -1;
    return 0;
}

static int texture(int fd, unsigned largeur, unsigned hauteur, unsigned motif, const struct canaux *canaux)
{
    struct bo_gpu_texture source = {largeur, hauteur, BO_GPU_FORMAT_BGRX8888, 0};
    struct bo_gpu_texture cible = {largeur, hauteur, BO_GPU_FORMAT_BGRX8888, 0};
    unsigned pas = largeur - 1;
    uint32_t *pixels = calloc((size_t)pas * hauteur, sizeof(*pixels));
    if (!pixels) return 1;
    int etape = 2;
    if (ioctl(fd, BO_GPU_CREE, &source) || ioctl(fd, BO_GPU_CREE, &cible)) goto fin;
    uint32_t distant = 0;
    if (echange(canaux, source.id, &distant)) goto fin;
    if (canaux && (!ioctl(fd, BO_GPU_DETRUIT, &distant) || errno != ENOENT)) goto fin;

    struct bo_gpu_transfert t = {source.id, 3, 5, largeur - 6, hauteur - 10, pas * 4,
                                  (uintptr_t)pixels};
    etape = 3;
    for (unsigned y = 0; y < t.hauteur; ++y)
        for (unsigned x = 0; x < t.largeur; ++x)
            pixels[(size_t)y * pas + x] = pixel(x, y, motif);
    if (ioctl(fd, BO_GPU_CHARGE, &t) || echange(canaux, 0, &distant)) goto fin;
    struct bo_gpu_blit b = {source.id, cible.id, 3, 5, 4, 7, t.largeur, t.hauteur};
    etape = 4;
    if (ioctl(fd, BO_GPU_BLIT, &b) || echange(canaux, 0, &distant)) goto fin;
    etape = 5;
    uint32_t *image = realloc(pixels, (size_t)largeur * hauteur * sizeof(*pixels));
    if (!image) goto fin;
    pixels = image;
    struct bo_gpu_transfert lecture = {cible.id, 0, 0, largeur, hauteur, largeur * 4,
                                        (uintptr_t)pixels};
    if (ioctl(fd, BO_GPU_RELIT, &lecture)) goto fin;
    etape = 6;
    for (unsigned y = 0; y < hauteur; ++y) {
        for (unsigned x = 0; x < largeur; ++x) {
            uint32_t attendu = x >= b.dx && x < b.dx + b.largeur
                             && y >= b.dy && y < b.dy + b.hauteur
                             ? pixel(x - b.dx, y - b.dy, motif) : 0;
            uint32_t obtenu = pixels[(size_t)y * largeur + x] & 0xffffff;
            if (obtenu != attendu) {
                printf("VIRTGPU_PIXEL x=%u y=%u lu=%06x attendu=%06x\n", x, y, obtenu, attendu);
                goto fin;
            }
        }
    }
    etape = 7;
    t.pixels = (uintptr_t)pixels;
    t.pas = 4;
    if (!ioctl(fd, BO_GPU_CHARGE, &t) || errno != EINVAL) goto fin;
    t.pas = largeur * 4;
    t.x = largeur - 1;
    if (!ioctl(fd, BO_GPU_CHARGE, &t) || errno != EINVAL) goto fin;
    etape = 8;
    if (ioctl(fd, BO_GPU_DETRUIT, &source.id) || ioctl(fd, BO_GPU_DETRUIT, &cible.id)) goto fin;
    etape = 0;
fin:
    free(pixels);
    if (etape) printf("VIRTGPU_TEXTURE_ECHEC %ux%u etape=%d errno=%d\n", largeur, hauteur, etape, errno);
    return etape;
}

static int multiprocessus(void)
{
    int aller[2], retour[2];
    if (pipe(aller) || pipe(retour)) return 1;
    pid_t enfant = fork();
    if (enfant < 0) return 2;
    int est_enfant = enfant == 0;
    close(aller[est_enfant]);
    close(retour[!est_enfant]);
    struct canaux canaux = est_enfant ? (struct canaux){aller[0], retour[1]}
                                      : (struct canaux){retour[0], aller[1]};
    int fd = open("/dev/dri/renderD128", O_RDWR);
    int resultat = fd < 0;
    for (unsigned tour = 0; !resultat && tour < 6; ++tour)
        resultat = texture(fd, 256, 256, 0x123400 + tour + est_enfant * 256, &canaux);
    close(canaux.lecture);
    close(canaux.ecriture);
    close(fd);
    if (est_enfant) _exit(resultat ? 1 : 0);
    int statut = 0;
    if (waitpid(enfant, &statut, 0) != enfant || !WIFEXITED(statut) || WEXITSTATUS(statut))
        return 3;
    if (resultat) return 4;
    puts("VIRTGPU_TEXTURES_ENTRELACEES_OK processus=2 cycles=12 phases=3 pixels=786432");
    return 0;
}

static int abandon(void)
{
    // Le parent tue le propriétaire alors que ses deux textures existent.
    // Une allocation suivante doit récupérer leurs places et leur quota.
    int lien[2];
    if (pipe(lien)) return 1;
    pid_t enfant = fork();
    if (enfant < 0) return 2;
    if (!enfant) {
        close(lien[0]);
        int fd = open("/dev/dri/renderD128", O_RDWR);
        struct bo_gpu_texture t = {256, 256, BO_GPU_FORMAT_BGRX8888, 0};
        if (fd < 0 || ioctl(fd, BO_GPU_CREE, &t)) _exit(1);
        uint32_t id = t.id;
        t.id = 0;
        if (ioctl(fd, BO_GPU_CREE, &t) || write(lien[1], &id, sizeof(id)) != sizeof(id))
            _exit(2);
        for (;;) pause();
    }
    close(lien[1]);
    uint32_t id;
    ssize_t lu = read(lien[0], &id, sizeof(id));
    close(lien[0]);
    if (kill(enfant, SIGKILL)) return 3;
    int statut = 0;
    if (waitpid(enfant, &statut, 0) != enfant || !WIFSIGNALED(statut) || lu != sizeof(id))
        return 4;
    int fd = open("/dev/dri/renderD128", O_RDWR);
    if (fd < 0 || !ioctl(fd, BO_GPU_DETRUIT, &id) || errno != ENOENT) return 5;
    int resultat = texture(fd, 1024, 1024, 0x214365, NULL);
    close(fd);
    if (resultat) return 6;
    puts("VIRTGPU_TEXTURES_ABORT_OK");
    return 0;
}

int main(int argc, char **argv)
{
    (void)argv;
    if (argc > 1) return multiprocessus() || abandon();
    int fd = open("/dev/dri/renderD128", O_RDWR);
    if (fd < 0) return 1;
    struct bo_gpu_texture invalide = {1025, 1, BO_GPU_FORMAT_BGRX8888, 0};
    if (!ioctl(fd, BO_GPU_CREE, &invalide) || errno != EINVAL) return 2;
    invalide.largeur = 1;
    invalide.format = 0xffffffff;
    if (!ioctl(fd, BO_GPU_CREE, &invalide) || errno != EINVAL) return 3;
    for (unsigned tour = 0; tour < 2; ++tour) {
        if (texture(fd, 64, 64, 0xcaf0 + tour, NULL) || texture(fd, 256, 256, 0xfade + tour, NULL)
            || texture(fd, 513, 137, 0xbaab + tour, NULL)
            || texture(fd, 1024, 1024, 0xabcd + tour, NULL))
            return 4;
    }
    close(fd);
    puts("VIRTGPU_TEXTURES_VARIABLES_OK cycles=8 pixels=2376978");
    return 0;
}
