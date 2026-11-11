/*
 * MIRE-GPU : client graphique ring 3 de verification du compositeur
 * (BOUCHAUD_GPU_COMPOSITION_V2).
 *
 * Chaque pixel ENCODE sa position dans la surface :
 *     R = y & 0xff, G = x & 0xff, B = 0x80 | ((y >> 8) << 3) | (x >> 8)
 * Une capture de l'ecran suffit donc a dire, pour chaque pixel affiche, quel
 * pixel de la surface il montre : orientation, decalage, lignes manquantes,
 * morceaux oublies. Un carre de 32x32 pixels blancs se deplace d'une case
 * par trame avec un dommage PARTIEL (le carre ancien et le nouveau) : cela
 * eprouve le transfert du seul rectangle abime.
 *
 *   musl-gcc -O2 -static-pie -fPIE -o mire-gpu mire-gpu.c
 *
 * Variables : BO_GUI_FD, BO_SURFACE_FD, BO_SURFACE_WIDTH/HEIGHT/STRIDE (posees
 * par le gestionnaire de fenetres, comme pour tout client). BO_MIRE_FAUTE_APRES=N
 * (banc de crash) : faute de page volontaire apres N trames.
 */
#define _GNU_SOURCE
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#define BO_MAGIC 0x55474F42u
#define BO_VERSION 1u
#define BO_GUI_HELLO 1u
#define BO_GUI_TITLE 3u
#define BO_GUI_FRAME 6u
#define BO_GUI_CONFIGURE 0x101u
#define BO_GUI_CLOSE 0x106u

static int gui_fd = -1;
static uint32_t seq = 1;
static uint32_t *surf;
static uint32_t aw, ah, stride, vw, vh;

static void put16(uint8_t *p, uint16_t n) { p[0] = n; p[1] = n >> 8; }
static void put32(uint8_t *p, uint32_t n) { p[0] = n; p[1] = n >> 8; p[2] = n >> 16; p[3] = n >> 24; }
static uint32_t get32(const uint8_t *p) { return p[0] | (p[1] << 8) | (p[2] << 16) | ((uint32_t)p[3] << 24); }
static uint32_t env(const char *s) { const char *v = getenv(s); return v ? (uint32_t)strtoul(v, 0, 10) : 0; }

static void envoie(uint16_t genre, const void *charge, uint32_t n) {
    uint8_t d[16 + 128];
    put32(d, BO_MAGIC); put16(d + 4, BO_VERSION); put16(d + 6, genre);
    put32(d + 8, n); put32(d + 12, seq++);
    memcpy(d + 16, charge, n);
    (void)write(gui_fd, d, 16 + n);
}

static uint32_t motif(uint32_t x, uint32_t y) {
    return ((y & 0xff) << 16) | ((x & 0xff) << 8) | 0x80 | ((y >> 8) << 3) | (x >> 8);
}

static void ligne(uint32_t y, uint32_t x0, uint32_t x1) {
    uint32_t *l = (uint32_t *)((uint8_t *)surf + (size_t)y * stride);
    for (uint32_t x = x0; x < x1; x++) l[x] = motif(x, y);
}

static void trame(uint32_t x, uint32_t y, uint32_t w, uint32_t h) {
    uint8_t p[24];
    put32(p, 1); put32(p + 4, 0); put32(p + 8, x); put32(p + 12, y);
    put32(p + 16, w); put32(p + 20, h);
    envoie(BO_GUI_FRAME, p, 24);
}

static void pompe(void) {
    static uint8_t rx[4096];
    for (;;) {
        struct pollfd f = {gui_fd, POLLIN, 0};
        if (poll(&f, 1, 0) <= 0) return;
        ssize_t n = read(gui_fd, rx, sizeof rx);
        if (n <= 0) return;
        for (ssize_t o = 0; o + 16 <= n;) {
            uint32_t len = get32(rx + o + 8);
            uint16_t genre = rx[o + 6] | (rx[o + 7] << 8);
            if (genre == BO_GUI_CLOSE) _exit(0);
            if (genre == BO_GUI_CONFIGURE && len >= 16 && o + 16 + 16 <= n) {
                uint32_t w = get32(rx + o + 16 + 4), h = get32(rx + o + 16 + 8);
                if (w && h && w <= aw && h <= ah) { vw = w; vh = h; }
            }
            o += 16 + len;
        }
    }
}

int main(void) {
    gui_fd = (int)env("BO_GUI_FD");
    int sfd = (int)env("BO_SURFACE_FD");
    aw = env("BO_SURFACE_WIDTH"); ah = env("BO_SURFACE_HEIGHT"); stride = env("BO_SURFACE_STRIDE");
    if (!gui_fd || !aw || !ah || stride < aw * 4) { fprintf(stderr, "MIRE_GPU environnement absent\n"); return 1; }
    surf = mmap(0, (size_t)stride * ah, PROT_READ | PROT_WRITE, MAP_SHARED, sfd, 0);
    if (surf == MAP_FAILED) { perror("MIRE_GPU mmap"); return 1; }
    vw = aw; vh = ah;
    uint8_t hello[8]; put32(hello, BO_VERSION); put32(hello + 4, (uint32_t)getpid());
    envoie(BO_GUI_HELLO, hello, 8);
    envoie(BO_GUI_TITLE, "Mire GPU", 8);
    for (uint32_t y = 0; y < ah; y++) ligne(y, 0, aw);
    pompe();
    trame(0, 0, vw, vh);
    fprintf(stderr, "MIRE_GPU_PRETE surface=%ux%u vue=%ux%u\n", aw, ah, vw, vh);
    uint32_t px = 0, py = 0, n = 0, faute_apres = env("BO_MIRE_FAUTE_APRES");
    for (;;) {
        pompe();
        /* Restaure l'ancien carre, pose le nouveau : dommage partiel. */
        uint32_t ax = px, ay = py;
        for (uint32_t y = ay; y < ay + 32 && y < ah; y++) ligne(y, ax, ax + 32 < aw ? ax + 32 : aw);
        px = (px + 8) % (vw > 40 ? vw - 32 : 1);
        py = (n / 16 * 8) % (vh > 40 ? vh - 32 : 1);
        for (uint32_t y = py; y < py + 32 && y < ah; y++) {
            uint32_t *l = (uint32_t *)((uint8_t *)surf + (size_t)y * stride);
            for (uint32_t x = px; x < px + 32 && x < aw; x++) l[x] = 0xffffff;
        }
        uint32_t x0 = ax < px ? ax : px, y0 = ay < py ? ay : py;
        uint32_t x1 = (ax > px ? ax : px) + 32, y1 = (ay > py ? ay : py) + 32;
        trame(x0, y0, x1 - x0, y1 - y0);
        n++;
        /* BOUCHAUD_BUREAU_RING3_2A : banc de crash. Une VRAIE faute de page
         * (pas une sortie propre) apres N trames, pour eprouver ce que fait le
         * compositeur d'une application qui meurt sous lui. */
        if (faute_apres && n == faute_apres) {
            fprintf(stderr, "MIRE_GPU_FAUTE_VOLONTAIRE trames=%u\n", n);
            *(volatile uint32_t *)0 = n;
        }
        if (n % 600 == 0) fprintf(stderr, "MIRE_GPU trames=%u carre=%u,%u\n", n, px, py);
        struct timespec t = {0, 16 * 1000 * 1000};
        nanosleep(&t, 0);
    }
}
