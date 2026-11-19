/* Linux virtgpu discovery ABI, using fixed-width fields as in virtgpu_drm.h. */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

struct param { uint64_t param, value; };
struct caps { uint32_t id, version; uint64_t addr; uint32_t size, pad; };
#define GETPARAM _IOWR('d', 0x43, struct param)
#define GET_CAPS _IOWR('d', 0x49, struct caps)
typedef uint32_t pixels_t[64];
#define BLIT_SONDE _IOR('B', 0xa0, pixels_t)

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
    return close(fd) != 0;
}
