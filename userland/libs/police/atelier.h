/*
 * ATELIER.H -- la police d'Atelier pour les applications ring 3
 * (BOUCHAUD_ATLAS_POLICE_V1).
 *
 * Le noyau publie dans /usr/share/atelier/glyphes-<px>.bin les glyphes qu'il
 * dessine LUI-MEME pour le bureau (fontdue, DejaVu Sans) : memes couvertures,
 * memes avances, meme ligne de base, meme melange au pixel pres
 * (`drivers/display/pixels.rs::melange`), meme gras (chaque pixel double vers
 * la droite). Une application qui dessine avec ce fichier ressemble donc au
 * reste d'Atelier, sans rasteriseur TrueType.
 *
 * En-tete seul, sans allocation apres le chargement :
 *
 *   struct atelier_police p;
 *   if (atelier_police_charge(&p, 14) != 0) ... repli
 *   atelier_texte(&p, surface, pas_octets, largeur, hauteur, x, y_haut,
 *                 "Texte UTF-8", 0xe2e8f0, gras);
 *   atelier_largeur(&p, "Texte UTF-8");
 */
#ifndef BOUCHAUD_ATELIER_POLICE_H
#define BOUCHAUD_ATELIER_POLICE_H

#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

struct atelier_glyphe {
    uint32_t code;
    int16_t avance, gauche, haut;
    uint16_t largeur, hauteur;
    const uint8_t *couverture;
};

struct atelier_police {
    int px, ascendante, nombre;
    uint8_t *octets;
    struct atelier_glyphe *glyphes;
    struct atelier_glyphe *latin1[256];
};

static inline uint16_t atelier__u16(const uint8_t *p) { return (uint16_t)(p[0] | (p[1] << 8)); }
static inline uint32_t atelier__u32(const uint8_t *p)
{ return p[0] | (p[1] << 8) | (p[2] << 16) | ((uint32_t)p[3] << 24); }

/* 0 si l'atlas de `px` est charge, -1 sinon (fichier absent ou invalide). */
static inline int atelier_police_charge(struct atelier_police *p, int px)
{
    memset(p, 0, sizeof *p);
    char chemin[64];
    snprintf(chemin, sizeof chemin, "/usr/share/atelier/glyphes-%d.bin", px);
    int fd = open(chemin, O_RDONLY);
    if (fd < 0) return -1;
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size < 16 || st.st_size > (8 << 20)) { close(fd); return -1; }
    size_t n = (size_t)st.st_size;
    p->octets = malloc(n);
    size_t lu = 0;
    while (p->octets && lu < n) {
        ssize_t r = read(fd, p->octets + lu, n - lu);
        if (r <= 0) break;
        lu += (size_t)r;
    }
    close(fd);
    const uint8_t *o = p->octets;
    if (!o || lu != n || memcmp(o, "BOGA", 4) != 0 || atelier__u16(o + 4) != 1) goto invalide;
    p->px = atelier__u16(o + 6);
    p->ascendante = (int16_t)atelier__u16(o + 8);
    p->nombre = atelier__u16(o + 10);
    if (16 + (size_t)p->nombre * 20 > n) goto invalide;
    p->glyphes = calloc((size_t)p->nombre, sizeof *p->glyphes);
    if (!p->glyphes) goto invalide;
    for (int i = 0; i < p->nombre; i++) {
        const uint8_t *e = o + 16 + (size_t)i * 20;
        struct atelier_glyphe *g = &p->glyphes[i];
        g->code = atelier__u32(e);
        g->avance = (int16_t)atelier__u16(e + 4);
        g->gauche = (int16_t)atelier__u16(e + 6);
        g->haut = (int16_t)atelier__u16(e + 8);
        g->largeur = atelier__u16(e + 10);
        g->hauteur = atelier__u16(e + 12);
        uint32_t dec = atelier__u32(e + 16);
        /* Bornes verifiees : un atlas corrompu ne fait pas lire hors du fichier. */
        if ((size_t)dec + (size_t)g->largeur * g->hauteur > n) goto invalide;
        g->couverture = o + dec;
        if (g->code < 256) p->latin1[g->code] = g;
    }
    return 0;
invalide:
    free(p->glyphes);
    free(p->octets);
    memset(p, 0, sizeof *p);
    return -1;
}

static inline const struct atelier_glyphe *atelier_glyphe(const struct atelier_police *p, uint32_t c)
{
    if (c < 256) return p->latin1[c] ? p->latin1[c] : p->latin1['?'];
    for (int i = 0; i < p->nombre; i++)
        if (p->glyphes[i].code == c) return &p->glyphes[i];
    return p->latin1['?'];
}

/* Decode un point de code UTF-8 ; avance *s. Octet invalide : rendu tel quel
 * (Latin-1), jamais de lecture au-dela du terminateur. */
static inline uint32_t atelier_utf8(const char **s)
{
    const uint8_t *u = (const uint8_t *)*s;
    uint32_t c = u[0];
    int n = c < 0x80 ? 0 : (c & 0xe0) == 0xc0 ? 1 : (c & 0xf0) == 0xe0 ? 2 : (c & 0xf8) == 0xf0 ? 3 : -1;
    if (n < 0) { (*s)++; return c; }
    c &= n == 0 ? 0x7f : (0x3f >> n);
    for (int i = 1; i <= n; i++) {
        if ((u[i] & 0xc0) != 0x80) { (*s)++; return u[0]; }
        c = (c << 6) | (u[i] & 0x3f);
    }
    *s += n + 1;
    return c;
}

static inline int atelier_largeur_n(const struct atelier_police *p, const char *s, size_t octets)
{
    int w = 0;
    const char *fin = s + octets;
    while (s < fin && *s) {
        const struct atelier_glyphe *g = atelier_glyphe(p, atelier_utf8(&s));
        if (g) w += g->avance;
    }
    return w;
}

static inline int atelier_largeur(const struct atelier_police *p, const char *s)
{ return atelier_largeur_n(p, s, strlen(s)); }

/* Le melange du bureau, au bit pres (`pixels::melange`). */
static inline uint32_t atelier_melange(uint32_t dst, uint32_t rgb, uint32_t a)
{
    uint32_t inv = 255 - a;
    uint32_t rb = (dst & 0x00ff00ff) * inv + (rgb & 0x00ff00ff) * a;
    uint32_t g = ((dst >> 8) & 0xff) * inv + ((rgb >> 8) & 0xff) * a;
    rb = ((rb + 0x00010001 + ((rb >> 8) & 0x00ff00ff)) >> 8) & 0x00ff00ff;
    g = (g + 1 + (g >> 8)) >> 8;
    return rb | (g << 8);
}

/* Dessine au plus `octets` octets de `s` ; `x_max` borne a droite (exclu).
 * Rend l'abscisse apres le texte. */
static inline int atelier_texte_n(const struct atelier_police *p, uint32_t *surf, uint32_t pas,
                                  uint32_t l, uint32_t h, int x, int y_haut, const char *s,
                                  size_t octets, uint32_t rgb, int gras, int x_max)
{
    if (!p->octets) return x;
    int ligne_base = y_haut + p->ascendante;
    const char *fin = s + octets;
    if (x_max > (int)l) x_max = (int)l;
    rgb &= 0x00ffffff;
    while (s < fin && *s) {
        const struct atelier_glyphe *g = atelier_glyphe(p, atelier_utf8(&s));
        if (!g) continue;
        if (x + g->avance > x_max) break;
        int gx = x + g->gauche, gy = ligne_base - g->haut;
        for (int ry = 0; ry < g->hauteur; ry++) {
            int py = gy + ry;
            if (py < 0 || py >= (int)h) continue;
            uint32_t *row = (uint32_t *)((uint8_t *)surf + (size_t)py * pas);
            for (int rx = 0; rx < g->largeur; rx++) {
                uint8_t a = g->couverture[ry * g->largeur + rx];
                if (!a) continue;
                for (int k = 0; k <= (gras ? 1 : 0); k++) {
                    int px = gx + rx + k;
                    if (px < 0 || px >= x_max) continue;
                    row[px] = a == 255 ? rgb : atelier_melange(row[px], rgb, a);
                }
            }
        }
        x += g->avance;
    }
    return x;
}

static inline int atelier_texte(const struct atelier_police *p, uint32_t *surf, uint32_t pas,
                                uint32_t l, uint32_t h, int x, int y_haut, const char *s,
                                uint32_t rgb, int gras)
{ return atelier_texte_n(p, surf, pas, l, h, x, y_haut, s, strlen(s), rgb, gras, (int)l); }

#endif
