#ifndef BOUCHAUD_GPU_H
#define BOUCHAUD_GPU_H

#include <stdint.h>
#include <sys/ioctl.h>

#define BO_GPU_FORMAT_BGRX8888 2u

struct bo_gpu_texture {
    uint32_t largeur, hauteur, format, id;
};

struct bo_gpu_transfert {
    uint32_t id, x, y, largeur, hauteur, pas;
    uint64_t pixels;
};

struct bo_gpu_blit {
    uint32_t source, cible, sx, sy, dx, dy, largeur, hauteur;
};

#define BO_GPU_CREE _IOWR('B', 0xa6, struct bo_gpu_texture)
#define BO_GPU_DETRUIT _IOW('B', 0xa2, uint32_t)
#define BO_GPU_CHARGE _IOW('B', 0xa7, struct bo_gpu_transfert)
#define BO_GPU_RELIT _IOW('B', 0xa8, struct bo_gpu_transfert)
#define BO_GPU_BLIT _IOW('B', 0xa9, struct bo_gpu_blit)

#endif
