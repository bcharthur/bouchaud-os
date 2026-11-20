/* Linux virtgpu discovery ABI, using fixed-width fields as in virtgpu_drm.h. */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>
#include <sys/wait.h>

struct param { uint64_t param, value; };
struct caps { uint32_t id, version; uint64_t addr; uint32_t size, pad; };
#define GETPARAM _IOWR('d', 0x43, struct param)
#define GET_CAPS _IOWR('d', 0x49, struct caps)
typedef uint32_t pixels_t[64];
#define BLIT_SONDE _IOR('B', 0xa0, pixels_t)

struct transfert { uint32_t id, pad; uint64_t pixels; };
struct blit { uint32_t source, cible; };
#define CREE _IOR('B', 0xa1, uint32_t)
#define DETRUIT _IOW('B', 0xa2, uint32_t)
#define CHARGE _IOW('B', 0xa3, struct transfert)
#define RELIT _IOW('B', 0xa4, struct transfert)
#define BLIT _IOW('B', 0xa5, struct blit)

static int textures(int fd, unsigned motif)
{
    struct blit b;
    uint32_t extra;
    if (ioctl(fd, CREE, &b.source) || ioctl(fd, CREE, &b.cible)) return 1;
    if (!ioctl(fd, CREE, &extra) || errno != ENOMEM) return 2;
    pixels_t entree, sortie;
    struct transfert t = {b.cible, 0, (uintptr_t)sortie};
    if (!ioctl(fd, RELIT, &t) || errno != EINVAL) return 3;
    for (unsigned i = 0; i < 64; i++)
        entree[i] = ((i * 0x010511) ^ motif) & 0x00ffffff;
    struct transfert chargement = {b.source, 0, (uintptr_t)entree};
    if (ioctl(fd, CHARGE, &chargement) || ioctl(fd, BLIT, &b)
        || ioctl(fd, RELIT, &t)) return 4;
    for (unsigned i = 0; i < 64; i++)
        if ((sortie[i] & 0x00ffffff) != entree[i]) return 5;
    if (ioctl(fd, DETRUIT, &b.source) || ioctl(fd, DETRUIT, &b.cible)) return 6;
    if (!ioctl(fd, RELIT, &t) || errno != ENOENT) return 7;
    return 0;
}

static int test_textures(int fd)
{
    uint32_t parent;
    if (!ioctl(fd, CREE, (void *)1) || errno != EFAULT || ioctl(fd, CREE, &parent))
        return 1;
    pid_t enfant = fork();
    if (enfant < 0) return 2;
    if (!enfant) {
        close(fd);
        int autre = open("/dev/dri/renderD128", O_RDWR);
        if (autre < 0 || !ioctl(autre, DETRUIT, &parent) || errno != ENOENT)
            _exit(2);
        for (unsigned i = 0; i < 4; ++i) {
            int r = textures(autre, 0xabc111 + i);
            if (r) _exit(10 + r);
        }
        _exit(close(autre) != 0);
    }
    int statut = -1;
    if (waitpid(enfant, &statut, 0) != enfant || !WIFEXITED(statut) || WEXITSTATUS(statut)) {
        printf("VIRTGPU_TEXTURES enfant statut=%d\n", statut);
        return 3;
    }
    if (ioctl(fd, DETRUIT, &parent)) return 4;
    for (unsigned i = 0; i < 4; ++i) {
        int r = textures(fd, 0x135790 + i);
        if (r) return 10 + r;
    }
    for (unsigned i = 0; i < 10; ++i) {
        int autre = open("/dev/dri/renderD128", O_RDWR);
        uint32_t abandonne;
        if (autre < 0 || ioctl(autre, CREE, &abandonne) || close(autre)) return 5;
    }
    return 0;
}

int main(int argc, char **argv)
{
    (void)argv;
    int fd = open("/dev/dri/renderD128", O_RDWR);
    if (argc > 1) {
        if (fd >= 0 || errno != ENOENT)
            return 1;
        puts("VIRTGPU_CAPS_ABSENT_OK");
        return 0;
    }
    struct stat st;
    if (fd < 0 || fstat(fd, &st) || !S_ISCHR(st.st_mode)
        || major(st.st_rdev) != 226 || minor(st.st_rdev) != 128)
        return 2;
    uint64_t mask = 0;
    struct param p = {7, (uintptr_t)&mask};
    if (ioctl(fd, GETPARAM, &p) || (mask != (1u << 1) && mask != (1u << 2)))
        return 3;
    uint64_t blob = 1;
    p.param = 3;
    p.value = (uintptr_t)&blob;
    if (ioctl(fd, GETPARAM, &p) || blob)
        return 4;
    uint8_t buf[2048] = {0};
    struct caps c = {mask == (1u << 1) ? 1 : 2, 0, (uintptr_t)buf, sizeof(buf), 0};
    while (c.version <= 8 && ioctl(fd, GET_CAPS, &c)) {
        if (errno != EINVAL)
            return 5;
        ++c.version;
    }
    if (c.version > 8)
        return 6;
    unsigned nonzero = 0;
    for (unsigned i = 0; i < sizeof(buf); ++i)
        nonzero += buf[i] != 0;
    if (!nonzero)
        return 7;
    unsigned version = c.version;
    c.addr = 1;
    if (!ioctl(fd, GET_CAPS, &c) || errno != EFAULT)
        return 8;
    c.addr = (uintptr_t)buf;
    c.version = 0xffffffff;
    if (!ioctl(fd, GET_CAPS, &c) || errno != EINVAL)
        return 9;
    if (!ioctl(fd, _IO('d', 0x42), 0) || errno != ENOTTY)
        return 10;
    pixels_t pixels;
    if (!ioctl(fd, BLIT_SONDE, (void *)1) || errno != EFAULT)
        return 11;
    if (ioctl(fd, BLIT_SONDE, pixels))
        return 12;
    for (unsigned y = 0; y < 8; ++y) {
        for (unsigned x = 0; x < 8; ++x) {
            uint32_t attendu = y >= 1 && y <= 3 ? 0x00800000 | ((y + 1) << 8) | x : 0;
            if ((pixels[y * 8 + x] & 0x00ffffff) != attendu)
                return 13;
        }
    }
    if (!ioctl(fd, BLIT_SONDE, pixels) || errno != EBUSY)
        return 14;
    puts("VIRTGPU_BLIT_RING3_OK pixels=64");
    printf("VIRTGPU_CAPS_OK id=%u version=%u nonzero=%u\n", c.id, version, nonzero);
    int textures = test_textures(fd);
    if (textures) {
        printf("VIRTGPU_TEXTURES_ECHEC code=%d errno=%d\n", textures, errno);
        return 20 + textures;
    }
    puts("VIRTGPU_TEXTURES_RING3_OK processus=2 blits=8 pixels_par_blit=64");
    return close(fd) != 0;
}
