#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include "police5x7.h"

/* BOGU v1, single shared bitmap, using the inherited desktop channel. */
enum { MAGIC = 0x55474f42, HELLO = 1, TITLE = 3, FRAME = 6, CONFIGURE = 0x101, CLOSE = 0x106 };
struct header { uint32_t magic; uint16_t version, type; uint32_t length, serial; };
_Static_assert(sizeof(struct header) == 16, "BOGU header layout");
static int gui;
static uint32_t sequence, width, height, max_width, max_height, stride;
static unsigned char *bitmap;

static int send_message(uint16_t type, void const *payload, uint32_t size)
{
    struct { struct header header; unsigned char payload[24]; } message = {
        .header = { MAGIC, 1, type, size, ++sequence },
    };
    memcpy(message.payload, payload, size);
    ssize_t result;
    do {
        result = write(gui, &message, sizeof message.header + size);
    } while (result < 0 && errno == EINTR);
    return result == (ssize_t)(sizeof message.header + size) ? 0 : -1;
}

static void label(uint32_t y, char const *text)
{
    for (uint32_t x = 8; *text && x + POLICE_L <= width; ++text, x += POLICE_L + 1)
        for (unsigned row = 0; row < POLICE_H && y + row < height; ++row)
            for (unsigned col = 0; col < POLICE_L; ++col)
                if (police_allume(*text, col, row))
                    ((uint32_t *)(bitmap + (size_t)(y + row) * stride))[x + col] = 0xeff3f8;
}

static void field(char const *record, char const *key, char *output, size_t size)
{
    char const *value = strstr(record, key);
    value = value ? value + strlen(key) : "N/A";
    size_t length = strcspn(value, " \n");
    snprintf(output, size, "%.*s", (int)length, value);
}

static int paint(void)
{
    for (uint32_t y = 0; y < height; ++y)
        for (uint32_t x = 0; x < width; ++x)
            ((uint32_t *)(bitmap + (size_t)y * stride))[x] = 0x181d25;
    label(8, "Moniteur systeme");
    FILE *file = fopen("/proc/bouchaud/metrics", "r");
    char record[1024], line[96], first[32], second[32];
    while (file && fgets(record, sizeof record, file)) {
        if (strncmp(record, "SYSTEM_METRICS ", 15) == 0) {
            field(record, " cpu_busy_pct=", first, sizeof first);
            snprintf(line, sizeof line, "CPU %s pour cent", first);
            label(28, line);
            field(record, " ram_used_bytes=", first, sizeof first);
            field(record, " ram_total_bytes=", second, sizeof second);
            snprintf(line, sizeof line, "RAM %s / %s octets", first, second);
            label(42, line);
        } else if (strncmp(record, "DISK_METRICS ", 13) == 0) {
            field(record, " ata_backing_read_bytes_per_sec=", first, sizeof first);
            snprintf(line, sizeof line, "ATA %s octets/s", first);
            label(56, line);
        } else if (strncmp(record, "GPU_METRICS ", 12) == 0) {
            field(record, " virgl_frames=", first, sizeof first);
            snprintf(line, sizeof line, "VIRGL %s trames - GPU N/A", first);
            label(70, line);
        }
    }
    if (file)
        fclose(file);
    else
        label(28, "Metriques indisponibles");
    uint32_t frame[] = { 1, 0, 0, 0, width, height };
    return send_message(FRAME, frame, sizeof frame);
}

static uint32_t setting(char const *name)
{
    char const *value = getenv(name);
    return value ? strtoul(value, NULL, 10) : 0;
}

int main(void)
{
    gui = setting("BO_GUI_FD");
    int surface = setting("BO_SURFACE_FD");
    max_width = setting("BO_SURFACE_WIDTH");
    max_height = setting("BO_SURFACE_HEIGHT");
    stride = setting("BO_SURFACE_STRIDE");
    if (!gui || !surface || !max_width || !max_height || max_width > stride / 4)
        return 1;
    bitmap = mmap(NULL, (size_t)stride * max_height, PROT_READ | PROT_WRITE, MAP_SHARED, surface, 0);
    if (bitmap == MAP_FAILED)
        return 1;

    uint32_t hello[] = { 1, getpid() };
    if (send_message(HELLO, hello, sizeof hello) < 0 || send_message(TITLE, "Moniteur ring 3", 15) < 0)
        return 1;

    unsigned char input[4096];
    size_t filled = 0;
    struct pollfd poll_gui = { .fd = gui, .events = POLLIN };
    for (;;) {
        int ready = poll(&poll_gui, 1, width ? 1000 : -1);
        if (ready < 0 && errno == EINTR)
            continue;
        if (ready < 0)
            return 1;
        if (!ready) {
            if (paint() < 0)
                return 1;
            continue;
        }
        ssize_t received = read(gui, input + filled, sizeof input - filled);
        if (received < 0 && (errno == EINTR || errno == EAGAIN))
            continue;
        if (received <= 0)
            return received < 0;
        filled += received;
        size_t offset = 0;
        while (filled - offset >= sizeof(struct header)) {
            struct header h;
            memcpy(&h, input + offset, sizeof h);
            if (h.magic != MAGIC || h.version != 1 || h.length > sizeof input - sizeof h)
                return 1;
            if (filled - offset - sizeof h < h.length)
                break;
            if (h.type == CLOSE)
                return 0;
            if (h.type == CONFIGURE && h.length >= 16) {
                uint32_t geometry[4];
                memcpy(geometry, input + offset + sizeof h, sizeof geometry);
                if (geometry[1] && geometry[2] && geometry[1] <= max_width && geometry[2] <= max_height
                    && (geometry[1] != width || geometry[2] != height)) {
                    width = geometry[1];
                    height = geometry[2];
                    if (paint() < 0)
                        return 1;
                }
            }
            offset += sizeof h + h.length;
        }
        memmove(input, input + offset, filled - offset);
        filled -= offset;
    }
}
