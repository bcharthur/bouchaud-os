#include "bouchaud-gpu.h"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

static uint32_t pixel(unsigned x, unsigned y, unsigned motif)
{
    return ((x * 31337) ^ (y * 12345) ^ motif) & 0xffffff;
}

static int texture(int fd, unsigned largeur, unsigned hauteur, unsigned motif)
{
    struct bo_gpu_texture source = {largeur, hauteur, BO_GPU_FORMAT_BGRX8888, 0};
    struct bo_gpu_texture cible = {largeur, hauteur, BO_GPU_FORMAT_BGRX8888, 0};
    unsigned pas = largeur - 1;
    uint32_t *pixels = calloc((size_t)pas * hauteur, sizeof(*pixels));
    if (!pixels) return 1;
    int etape = 2;
    if (ioctl(fd, BO_GPU_CREE, &source) || ioctl(fd, BO_GPU_CREE, &cible)) goto fin;
    struct bo_gpu_transfert t = {source.id, 3, 5, largeur - 6, hauteur - 10, pas * 4,
                                  (uintptr_t)pixels};
    etape = 3;
    for (unsigned y = 0; y < t.hauteur; ++y)
        for (unsigned x = 0; x < t.largeur; ++x)
            pixels[(size_t)y * pas + x] = pixel(x, y, motif);
    if (ioctl(fd, BO_GPU_CHARGE, &t)) goto fin;
    struct bo_gpu_blit b = {source.id, cible.id, 3, 5, 4, 7, t.largeur, t.hauteur};
    etape = 4;
    if (ioctl(fd, BO_GPU_BLIT, &b)) goto fin;
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

int main(void)
{
    int fd = open("/dev/dri/renderD128", O_RDWR);
    if (fd < 0) return 1;
    struct bo_gpu_texture invalide = {1025, 1, BO_GPU_FORMAT_BGRX8888, 0};
    if (!ioctl(fd, BO_GPU_CREE, &invalide) || errno != EINVAL) return 2;
    invalide.largeur = 1;
    invalide.format = 0xffffffff;
    if (!ioctl(fd, BO_GPU_CREE, &invalide) || errno != EINVAL) return 3;
    for (unsigned tour = 0; tour < 2; ++tour) {
        if (texture(fd, 64, 64, 0xcaf0 + tour) || texture(fd, 256, 256, 0xfade + tour)
            || texture(fd, 513, 137, 0xbaab + tour)
            || texture(fd, 1024, 1024, 0xabcd + tour))
            return 4;
    }
    close(fd);
    puts("VIRTGPU_TEXTURES_VARIABLES_OK cycles=8 pixels=2376978");
    return 0;
}
