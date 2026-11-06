/*
 * Bouchaud OS ring3 GUI adapter for Doomgeneric.
 * Copyright (c) 2026 — GPL-2.0-or-later, compatible with Doomgeneric.
 * Uses only Linux ABI syscalls and the BO GUI protocol. No X11/SDL/Qt.
 */
#define _GNU_SOURCE
#include "doomgeneric.h"
#include "doomkeys.h"
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define BO_MAGIC 0x55474F42u
#define BO_VERSION 1u
#define BO_GUI_HELLO 1u
#define BO_GUI_TITLE 3u
#define BO_GUI_FRAME 6u
#define BO_GUI_CONFIGURE 0x101u
#define BO_GUI_KEY 0x103u
#define BO_GUI_CLOSE 0x106u
#define BO_MAX_PAYLOAD 4096u
#define QUEUE_SIZE 128u

static int gui_fd = -1;
static uint32_t* surface = NULL;
static uint32_t alloc_w = 0, alloc_h = 0, stride = 0;
static uint32_t view_w = 0, view_h = 0;
static uint32_t sequence = 1;
static uint8_t rx[8192];
static size_t rx_used = 0;
static uint16_t key_q[QUEUE_SIZE];
static unsigned q_read = 0, q_write = 0;
static int running = 1;
static uint32_t last_mods = 0;

static void put16(uint8_t *p, uint16_t n) {
    p[0] = (uint8_t)n;
    p[1] = (uint8_t)(n >> 8);
}
static void put32(uint8_t *p, uint32_t n) {
    p[0] = (uint8_t)n;
    p[1] = (uint8_t)(n >> 8);
    p[2] = (uint8_t)(n >> 16);
    p[3] = (uint8_t)(n >> 24);
}
static uint16_t get16(const uint8_t *p) {
    return (uint16_t)p[0] | ((uint16_t)p[1] << 8);
}
static uint32_t get32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}
static uint32_t env_u32(const char *s, uint32_t fallback) {
    const char *v = getenv(s);
    if (!v || !*v) return fallback;
    char *end = NULL;
    unsigned long n = strtoul(v, &end, 10);
    if (!end || *end || n > 0xffffffffUL) return fallback;
    return (uint32_t)n;
}

/* A single write per packet: BO transport demands atomic header+payload. */
static void send_message(uint16_t kind, const void *payload, uint32_t length) {
    if (gui_fd < 0 || length > BO_MAX_PAYLOAD) return;
    uint8_t data[16 + 256];
    if (length > sizeof(data) - 16) return;
    put32(data, BO_MAGIC);
    put16(data + 4, BO_VERSION);
    put16(data + 6, kind);
    put32(data + 8, length);
    put32(data + 12, sequence++);
    if (length) memcpy(data + 16, payload, length);
    (void)write(gui_fd, data, 16 + length);
}
static void push_key(int pressed, unsigned char key) {
    if (!key) return;
    unsigned n = (q_write + 1) % QUEUE_SIZE;
    if (n == q_read) return;
    key_q[q_write] = (uint16_t)(((pressed ? 1 : 0) << 8) | key);
    q_write = n;
}
static void sync_mods(uint32_t mods) {
    static const uint8_t keys[3] = {KEY_RSHIFT, KEY_FIRE, KEY_RALT};
    for (unsigned bit = 0; bit < 3; bit++) {
        const uint32_t mask = 1u << bit;
        if ((last_mods & mask) != (mods & mask))
            push_key((mods & mask) != 0, keys[bit]);
    }
    last_mods = mods;
}
static unsigned char to_doom_key(uint32_t code, uint32_t unicode) {
    switch (code) {
        case 1: return KEY_ENTER;
        case 2: return KEY_BACKSPACE;
        case 3: return KEY_TAB;
        case 4: return KEY_UPARROW;
        case 5: return KEY_DOWNARROW;
        case 6: return KEY_LEFTARROW;
        case 7: return KEY_RIGHTARROW;
        case 8: return KEY_ESCAPE;
        case 9: return KEY_HOME;
        case 10: return KEY_END;
        case 11: return KEY_PGUP;
        case 12: return KEY_PGDN;
        case 13: return KEY_DEL;
        case 14: return KEY_INS;
        case 15:
            if (unicode >= 1 && unicode <= 12) return (uint8_t)(KEY_F1 + unicode - 1);
            break;
        default: break;
    }
    if (unicode == 32) return KEY_USE;
    if (unicode == '\r' || unicode == '\n') return KEY_ENTER;
    if (unicode > 0 && unicode < 128) return (uint8_t)tolower((int)unicode);
    return 0;
}
static void handle_packet(uint16_t kind, const uint8_t *body, uint32_t len) {
    if (kind == BO_GUI_CONFIGURE && len >= 16) {
        uint32_t w = get32(body + 4), h = get32(body + 8);
        if (w && w <= alloc_w && h && h <= alloc_h) {
            view_w = w;
            view_h = h;
        }
    } else if (kind == BO_GUI_KEY && len >= 20) {
        uint32_t code = get32(body + 4);
        uint32_t mods = get32(body + 8);
        uint32_t ch = get32(body + 12);
        int down = get32(body + 16) != 0;
        sync_mods(mods);
        push_key(down, to_doom_key(code, ch));
    } else if (kind == BO_GUI_CLOSE) {
        running = 0;
    }
}
static void consume_rx(void) {
    size_t offset = 0;
    while (rx_used - offset >= 16) {
        const uint8_t *h = rx + offset;
        if (get32(h) != BO_MAGIC || get16(h + 4) != BO_VERSION) {
            /* Tolerate one stray byte without losing the rest of a stream. */
            offset++;
            continue;
        }
        uint32_t n = get32(h + 8);
        if (n > BO_MAX_PAYLOAD) { offset++; continue; }
        if (rx_used - offset < 16 + n) break;
        handle_packet(get16(h + 6), h + 16, n);
        offset += 16 + n;
    }
    if (offset) {
        memmove(rx, rx + offset, rx_used - offset);
        rx_used -= offset;
    }
}
static void pump_gui(void) {
    if (gui_fd < 0) return;
    for (unsigned i = 0; i < 16; i++) {
        struct pollfd pfd = {.fd = gui_fd, .events = POLLIN};
        int n = poll(&pfd, 1, 0);
        if (n <= 0 || !(pfd.revents & POLLIN)) break;
        if (rx_used == sizeof(rx)) { rx_used = 0; }
        ssize_t got = read(gui_fd, rx + rx_used, sizeof(rx) - rx_used);
        if (got <= 0) break;
        rx_used += (size_t)got;
        consume_rx();
    }
    if (!running) {
        /* WM requested closing the external process. */
        _exit(0);
    }
}
void DG_Init(void) {
    gui_fd = (int)env_u32("BO_GUI_FD", UINT32_MAX);
    int sfd = (int)env_u32("BO_SURFACE_FD", UINT32_MAX);
    alloc_w = env_u32("BO_SURFACE_WIDTH", 0);
    alloc_h = env_u32("BO_SURFACE_HEIGHT", 0);
    stride = env_u32("BO_SURFACE_STRIDE", 0);
    if (gui_fd < 0 || sfd < 0 || !alloc_w || !alloc_h ||
        alloc_w > 8192 || alloc_h > 8192 || stride < alloc_w * 4 ||
        stride > 8192 * 4u) {
        fprintf(stderr, "[DOOM] BO GUI environment missing or invalid\n");
        exit(1);
    }
    view_w = alloc_w;
    view_h = alloc_h;
    size_t bytes = (size_t)stride * alloc_h;
    surface = mmap(NULL, bytes, PROT_READ | PROT_WRITE, MAP_SHARED, sfd, 0);
    if (surface == MAP_FAILED) {
        perror("[DOOM] mmap(BO_SURFACE_FD)");
        exit(1);
    }
    uint8_t hello[8];
    put32(hello + 0, BO_VERSION);
    put32(hello + 4, (uint32_t)getpid());
    send_message(BO_GUI_HELLO, hello, sizeof(hello));
    DG_SetWindowTitle("DOOM - Freedoom Phase 1");
    fprintf(stderr, "[DOOM] BO_GUI_FD=%d size=%ux%u stride=%u active\n",
            gui_fd, alloc_w, alloc_h, stride);
}
void DG_DrawFrame(void) {
    pump_gui();
    if (!surface || !DG_ScreenBuffer) return;
    uint32_t w = view_w, h = view_h;
    if (w > alloc_w) w = alloc_w;
    if (h > alloc_h) h = alloc_h;
    if (!w || !h) return;
    /* Aspect-preserving nearest-neighbour scaling, black letterbox. */
    const uint32_t src_w = DOOMGENERIC_RESX, src_h = DOOMGENERIC_RESY;
    uint32_t draw_w = w, draw_h = (uint32_t)((uint64_t)w * src_h / src_w);
    if (draw_h > h) {
        draw_h = h;
        draw_w = (uint32_t)((uint64_t)h * src_w / src_h);
    }
    if (!draw_w || !draw_h) return;
    const uint32_t left = (w - draw_w) / 2u, top = (h - draw_h) / 2u;
    uint8_t *base = (uint8_t*)surface;
    for (uint32_t y = 0; y < h; y++) {
        uint32_t *dst = (uint32_t *)(base + (size_t)y * stride);
        if (y < top || y >= top + draw_h) {
            memset(dst, 0, (size_t)w * 4);
            continue;
        }
        uint32_t sy = (uint32_t)(((uint64_t)(y - top) * src_h) / draw_h);
        memset(dst, 0, (size_t)left * 4);
        for (uint32_t x = 0; x < draw_w; x++) {
            uint32_t sx = (uint32_t)(((uint64_t)x * src_w) / draw_w);
            dst[left + x] = DG_ScreenBuffer[(size_t)sy * src_w + sx];
        }
        memset(dst + left + draw_w, 0, (size_t)(w - left - draw_w) * 4);
    }
    uint8_t payload[24];
    put32(payload + 0, 1); /* window */
    put32(payload + 4, 0); /* buffer 0 (V1) */
    put32(payload + 8, 0); /* x */
    put32(payload + 12, 0); /* y */
    put32(payload + 16, w);
    put32(payload + 20, h);
    send_message(BO_GUI_FRAME, payload, sizeof(payload));
}
void DG_SleepMs(uint32_t ms) {
    struct timespec ts = {.tv_sec = ms / 1000u, .tv_nsec = (long)(ms % 1000u) * 1000000L};
    nanosleep(&ts, NULL);
}
uint32_t DG_GetTicksMs(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t) != 0) return 0;
    return (uint32_t)((uint64_t)t.tv_sec * 1000u + t.tv_nsec / 1000000u);
}
int DG_GetKey(int *pressed, unsigned char *key) {
    pump_gui();
    if (q_read == q_write) return 0;
    uint16_t item = key_q[q_read];
    q_read = (q_read + 1) % QUEUE_SIZE;
    *pressed = (item >> 8) != 0;
    *key = (uint8_t)item;
    return 1;
}
void DG_SetWindowTitle(const char *title) {
    if (title && *title) {
        size_t n = strnlen(title, 96);
        send_message(BO_GUI_TITLE, title, (uint32_t)n);
    }
}
int main(int argc, char **argv) {
    doomgeneric_Create(argc, argv);
    for (;;) doomgeneric_Tick();
    return 0;
}
