#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <unistd.h>

#define VERSION_VENUS _IOR('B', 0xaa, uint32_t)
#define DEMAPPE _IO('B', 0xab)
struct requete { uint32_t octets, version; };
#define VERSION_TAILLE _IOWR('B', 0xac, struct requete)

int main(int argc, char **argv)
{
    (void)argv;
    int fd = open("/dev/dri/renderD128", O_RDWR);
    if (fd < 0) return 1;
    if (!ioctl(fd, VERSION_VENUS, (void *)1) || errno != EFAULT) return 2;
    uint32_t reference = 0;
    for (unsigned i = 0; i < 3; ++i) {
        uint32_t version = 0;
        if (ioctl(fd, VERSION_VENUS, &version)) {
            perror("VERSION_VENUS");
            return 3;
        }
        if (version >> 22 != 1 || (i && version != reference)) return 4;
        reference = version;
    }
    printf("VIRTGPU_VENUS_VERSION_RING3_OK appels=3 api=%" PRIu32 ".%" PRIu32 ".%" PRIu32 "\n",
           reference >> 22, (reference >> 12) & 0x3ff, reference & 0xfff);
    if (argc > 1) {
        if (ioctl(fd, DEMAPPE)) {
            perror("VENUS_DEMAPPE");
            return 6;
        }
        uint32_t version = 0;
        if (ioctl(fd, VERSION_VENUS, &version) || version != reference) return 7;
        if (ioctl(fd, DEMAPPE)) return 8;
        const uint32_t tailles[] = {4096, 65536, 1048576, 4096};
        for (unsigned i = 0; i < sizeof(tailles) / sizeof(tailles[0]); ++i) {
            struct requete invalide = {4097, 0};
            if (!ioctl(fd, VERSION_TAILLE, &invalide) || errno != EINVAL) return 9;
            invalide.octets = 1048576 + 4096;
            if (!ioctl(fd, VERSION_TAILLE, &invalide) || errno != EINVAL) return 10;
            struct requete valide = {tailles[i], 0};
            if (ioctl(fd, VERSION_TAILLE, &valide) || valide.version != reference) return 11;
            invalide.octets = tailles[i] == 4096 ? 65536 : 4096;
            if (!ioctl(fd, VERSION_TAILLE, &invalide) || errno != EINVAL) return 12;
            valide.version = 0;
            if (ioctl(fd, VERSION_TAILLE, &valide) || valide.version != reference) return 13;
            if (ioctl(fd, DEMAPPE)) return 14;
        }
        puts("VIRTGPU_VENUS_TAILLES_OK cycles=4");
        puts("VIRTGPU_VENUS_DEMAPPE_OK");
    }
    return close(fd) != 0;
}
