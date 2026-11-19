#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <unistd.h>

#define VERSION_VENUS _IOR('B', 0xaa, uint32_t)

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
        if (ioctl(fd, _IO('B', 0xab))) {
            perror("VENUS_DEMAPPE");
            return 6;
        }
        uint32_t version = 0;
        if (ioctl(fd, VERSION_VENUS, &version) || version != reference) return 7;
        puts("VIRTGPU_VENUS_DEMAPPE_OK");
    }
    return close(fd) != 0;
}
