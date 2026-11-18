/*
 * CALCULATRICE-GUI -- la calculatrice du bureau, en APPLICATION ring 3
 * (BOUCHAUD_CALCULATRICE_RING3_V1).
 *
 * Elle remplace `src/gui/apps/calculator.rs`, qui dessinait et evaluait dans
 * le noyau. C'est un client du protocole GUI (BOGU) ordinaire, comme DOOM ou
 * Ladybird : surface partagee + canal, entrees recues du gestionnaire de
 * fenetres (bureau noyau OU compositeur ring 3, sans difference pour elle).
 *
 *   musl-gcc -O2 -static-pie -fPIE -I userland/libs/police \
 *       -o calculatrice-gui userland/apps/calculatrice/calculatrice-gui.c
 *
 * Entrees : clic sur une touche, ou clavier (chiffres, + - * / ( ) . ,
 * Entree ou '=' pour evaluer, Retour arriere, 'c' ou Echap pour effacer).
 * Le processus DORT sur son canal entre deux evenements (poll bloquant).
 *
 * Journal (sortie d'erreur, lue par les bancs) :
 *   CALC_GUI_PRETE pid= l= h=        apres la premiere trame
 *   CALC_GUI_TOUCHE_RECT label= x= y= l= h=   (geometrie, a chaque taille)
 *   CALC_GUI_TOUCHE label= source=clic|clavier
 *   CALC_GUI_RESULTAT expr= valeur=
 *   CALC_GUI_TAILLE l= h=
 */
#define _GNU_SOURCE
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include "police5x7.h"

#define BO_MAGIC 0x55474F42u
#define BO_VERSION 1u
#define BO_GUI_HELLO 1u
#define BO_GUI_TITLE 3u
#define BO_GUI_FRAME 6u
#define BO_GUI_CONFIGURE 0x101u
#define BO_GUI_KEY 0x103u
#define BO_GUI_POINTER 0x104u
#define BO_GUI_CLOSE 0x106u

static int gui_fd = -1;
static uint32_t seq = 1;
static uint32_t *surf;
static uint32_t aw, ah, stride, vw, vh;

static void put16(uint8_t *p, uint16_t n) { p[0] = n; p[1] = n >> 8; }
static void put32(uint8_t *p, uint32_t n) { p[0] = n; p[1] = n >> 8; p[2] = n >> 16; p[3] = n >> 24; }
static uint32_t get32(const uint8_t *p) { return p[0] | (p[1] << 8) | (p[2] << 16) | ((uint32_t)p[3] << 24); }
static uint32_t env(const char *s) { const char *v = getenv(s); return v ? (uint32_t)strtoul(v, 0, 10) : 0; }

static void envoie(uint16_t genre, const void *charge, uint32_t n)
{
    uint8_t d[16 + 128];
    if (n > 128) n = 128;
    put32(d, BO_MAGIC); put16(d + 4, BO_VERSION); put16(d + 6, genre);
    put32(d + 8, n); put32(d + 12, seq++);
    memcpy(d + 16, charge, n);
    (void)write(gui_fd, d, 16 + n);
}

/* --- etat ---------------------------------------------------------------- */

#define EXPR_MAX 48
static char expr[EXPR_MAX + 1];

static const char *const touches[20] = {
    "C", "(", ")", "/",
    "7", "8", "9", "*",
    "4", "5", "6", "-",
    "1", "2", "3", "+",
    "0", ".", "=", "<",
};

#define PAD 6u
#define AFF_H 64u

static void rect_touche(int i, uint32_t *x, uint32_t *y, uint32_t *l, uint32_t *h)
{
    uint32_t col = (uint32_t)i % 4, rang = (uint32_t)i / 4;
    uint32_t haut = AFF_H + 2 * PAD;
    uint32_t cl = vw > 5 * PAD ? (vw - 5 * PAD) / 4 : 1;
    uint32_t ch = vh > haut + 6 * PAD ? (vh - haut - 6 * PAD) / 5 : 1;
    *x = PAD + col * (cl + PAD);
    *y = haut + PAD + rang * (ch + PAD);
    *l = cl; *h = ch;
}

/* --- evaluation (descente recursive, flottants) -------------------------- */

static const char *pos;
static int erreur;

static void blancs(void) { while (*pos == ' ') pos++; }
static double somme(void);

static double facteur(void)
{
    blancs();
    if (*pos == '-') { pos++; return -facteur(); }
    if (*pos == '+') { pos++; return facteur(); }
    if (*pos == '(') {
        pos++;
        double v = somme();
        blancs();
        if (*pos == ')') pos++; else erreur = 1;
        return v;
    }
    char *fin;
    double v = strtod(pos, &fin);
    if (fin == pos) { erreur = 1; return 0; }
    pos = fin;
    return v;
}

static double produit(void)
{
    double v = facteur();
    for (;;) {
        blancs();
        if (*pos == '*') { pos++; v *= facteur(); }
        else if (*pos == '/') {
            pos++;
            double d = facteur();
            if (d == 0) { erreur = 1; return 0; }
            v /= d;
        } else return v;
    }
}

static double somme(void)
{
    double v = produit();
    for (;;) {
        blancs();
        if (*pos == '+') { pos++; v += produit(); }
        else if (*pos == '-') { pos++; v -= produit(); }
        else return v;
    }
}

static void evalue(void)
{
    char avant[EXPR_MAX + 1];
    memcpy(avant, expr, sizeof avant);
    pos = expr[0] ? expr : "0";
    erreur = 0;
    double v = somme();
    blancs();
    if (*pos) erreur = 1;
    if (erreur) snprintf(expr, sizeof expr, "Erreur");
    else snprintf(expr, sizeof expr, "%.10g", v);
    fprintf(stderr, "CALC_GUI_RESULTAT expr=%s valeur=%s\n", avant[0] ? avant : "0", expr);
}

static void applique(const char *label, const char *source)
{
    fprintf(stderr, "CALC_GUI_TOUCHE label=%s source=%s\n", label, source);
    if (!strcmp(label, "C")) { expr[0] = 0; return; }
    if (!strcmp(label, "<")) { size_t n = strlen(expr); if (n) expr[n - 1] = 0; return; }
    if (!strcmp(label, "=")) { evalue(); return; }
    if (!strcmp(expr, "Erreur")) expr[0] = 0;
    size_t n = strlen(expr);
    if (n < EXPR_MAX) { expr[n] = label[0]; expr[n + 1] = 0; }
}

/* --- dessin -------------------------------------------------------------- */

static void pixel(uint32_t x, uint32_t y, uint32_t c)
{
    if (x < vw && y < vh) ((uint32_t *)((uint8_t *)surf + (size_t)y * stride))[x] = c;
}

static void boite(uint32_t x, uint32_t y, uint32_t l, uint32_t h, uint32_t c)
{
    for (uint32_t j = y; j < y + h && j < vh; j++) {
        uint32_t *ligne = (uint32_t *)((uint8_t *)surf + (size_t)j * stride);
        for (uint32_t i = x; i < x + l && i < vw; i++) ligne[i] = c;
    }
}

static void texte(uint32_t x, uint32_t y, const char *t, unsigned e, uint32_t c)
{
    for (; *t; t++, x += (POLICE_L + 1) * e)
        for (unsigned gy = 0; gy < POLICE_H; gy++)
            for (unsigned gx = 0; gx < POLICE_L; gx++)
                if (police_allume(*t, gx, gy))
                    boite(x + gx * e, y + gy * e, e, e, c);
}

static void trame(void)
{
    uint8_t p[24];
    put32(p, 1); put32(p + 4, 0); put32(p + 8, 0); put32(p + 12, 0);
    put32(p + 16, vw); put32(p + 20, vh);
    envoie(BO_GUI_FRAME, p, 24);
}

static void dessine(void)
{
    boite(0, 0, vw, vh, 0x1f2023);
    boite(PAD, PAD, vw > 2 * PAD ? vw - 2 * PAD : 0, AFF_H, 0x0f1011);
    const char *montre = expr[0] ? expr : "0";
    unsigned e = 4;
    while (e > 1 && police_largeur(montre, e) + 4 * PAD > vw) e--;
    uint32_t tl = police_largeur(montre, e);
    uint32_t tx = vw > tl + 2 * PAD + 6 ? vw - tl - 2 * PAD - 6 : PAD;
    texte(tx, PAD + (AFF_H - POLICE_H * e) / 2, montre, e, 0x8ab4f8);
    for (int i = 0; i < 20; i++) {
        uint32_t x, y, l, h;
        rect_touche(i, &x, &y, &l, &h);
        const char *t = touches[i];
        uint32_t fond = !strcmp(t, "=") ? 0x1a73e8 : 0x3c4043;
        uint32_t encre = !strcmp(t, "=") ? 0xffffff
                       : (!strcmp(t, "C") || !strcmp(t, "<")) ? 0xff6b6b
                       : strchr("/*-+", t[0]) ? 0xfdd663 : 0xe8eaed;
        boite(x, y, l, h, fond);
        for (uint32_t k = 0; k < l; k++) { pixel(x + k, y, 0x55585c); pixel(x + k, y + h - 1, 0x55585c); }
        for (uint32_t k = 0; k < h; k++) { pixel(x, y + k, 0x55585c); pixel(x + l - 1, y + k, 0x55585c); }
        unsigned et = 3;
        texte(x + (l - police_largeur(t, et)) / 2, y + (h - POLICE_H * et) / 2, t, et, encre);
    }
    trame();
}

static void publie_geometrie(void)
{
    for (int i = 0; i < 20; i++) {
        uint32_t x, y, l, h;
        rect_touche(i, &x, &y, &l, &h);
        fprintf(stderr, "CALC_GUI_TOUCHE_RECT label=%s x=%u y=%u l=%u h=%u\n", touches[i], x, y, l, h);
    }
}

/* --- entrees ------------------------------------------------------------- */

static const char *touche_clavier(uint32_t u)
{
    static char un[2];
    if ((u >= '0' && u <= '9') || strchr("+-*/().", (int)u)) { un[0] = (char)u; un[1] = 0; return u ? un : 0; }
    if (u == ',') return ".";
    if (u == '=' || u == '\r' || u == '\n') return "=";
    if (u == 8 || u == 127) return "<";
    if (u == 'c' || u == 'C' || u == 27) return "C";
    return 0;
}

/* Traite les messages en attente ; rend 1 s'il faut redessiner. */
static int pompe(void)
{
    /* Le canal est un flux d'octets (socketpair) : une lecture peut couper
     * un message. Les octets non consommes sont GARDES et completes par la
     * lecture suivante (revue #336) ; seuls les messages entiers sont lus. */
    static uint8_t rx[8192];
    static size_t plein;
    static uint32_t boutons_prec;
    int redessine = 0;
    ssize_t lu = read(gui_fd, rx + plein, sizeof rx - plein);
    if (lu <= 0) _exit(0);   /* canal ferme : le gestionnaire est parti */
    plein += (size_t)lu;
    size_t o = 0;
    for (; o + 16 <= plein;) {
        uint32_t len = get32(rx + o + 8);
        uint16_t genre = rx[o + 6] | (rx[o + 7] << 8);
        const uint8_t *c = rx + o + 16;
        if (len > sizeof rx - 16) { plein = 0; return redessine; }   /* en-tete invalide : resynchroniser */
        if (o + 16 + len > plein) break;   /* message incomplet : attendre la suite */
        if (genre == BO_GUI_CLOSE) {
            fprintf(stderr, "CALC_GUI_FERMEE\n");
            _exit(0);
        }
        if (genre == BO_GUI_KEY && len >= 20 && get32(c + 16)) {
            /* Code logique (protocole GUI) : 1 Entree, 2 Retour arriere,
             * 8 Echap ; 0 = caractere, porte par `unicode`. */
            uint32_t code = get32(c + 4);
            const char *t = code == 1 ? "=" : code == 2 ? "<" : code == 8 ? "C"
                          : code == 0 ? touche_clavier(get32(c + 12)) : 0;
            if (t) { applique(t, "clavier"); redessine = 1; }
        }
        if (genre == BO_GUI_POINTER && len >= 16) {
            int32_t x = (int32_t)get32(c + 4), y = (int32_t)get32(c + 8);
            uint32_t b = get32(c + 12);
            /* Un clic = l'appui du bouton gauche dans une touche. */
            if ((b & 1) && !(boutons_prec & 1) && x >= 0 && y >= 0) {
                for (int i = 0; i < 20; i++) {
                    uint32_t tx, ty, tl, th;
                    rect_touche(i, &tx, &ty, &tl, &th);
                    if ((uint32_t)x >= tx && (uint32_t)x < tx + tl && (uint32_t)y >= ty && (uint32_t)y < ty + th) {
                        applique(touches[i], "clic");
                        redessine = 1;
                        break;
                    }
                }
            }
            boutons_prec = b;
        }
        if (genre == BO_GUI_CONFIGURE && len >= 16) {
            uint32_t w = get32(c + 4), h = get32(c + 8);
            if (w && h && w <= aw && h <= ah && (w != vw || h != vh)) {
                vw = w; vh = h;
                fprintf(stderr, "CALC_GUI_TAILLE l=%u h=%u\n", vw, vh);
                publie_geometrie();
                redessine = 1;
            }
        }
        o += 16 + len;
    }
    memmove(rx, rx + o, plein - o);
    plein -= o;
    return redessine;
}

int main(void)
{
    gui_fd = (int)env("BO_GUI_FD");
    int sfd = (int)env("BO_SURFACE_FD");
    aw = env("BO_SURFACE_WIDTH"); ah = env("BO_SURFACE_HEIGHT"); stride = env("BO_SURFACE_STRIDE");
    if (!gui_fd || !aw || !ah || stride < aw * 4) { fprintf(stderr, "CALC_GUI environnement absent\n"); return 1; }
    surf = mmap(0, (size_t)stride * ah, PROT_READ | PROT_WRITE, MAP_SHARED, sfd, 0);
    if (surf == MAP_FAILED) { perror("CALC_GUI mmap"); return 1; }
    vw = aw < 320 ? aw : 320; vh = ah < 420 ? ah : 420;
    uint8_t hello[8]; put32(hello, BO_VERSION); put32(hello + 4, (uint32_t)getpid());
    envoie(BO_GUI_HELLO, hello, 8);
    envoie(BO_GUI_TITLE, "Calculatrice", 12);
    /* La zone utile arrive par le premier Configure, deja en file. */
    struct pollfd f = { gui_fd, POLLIN, 0 };
    if (poll(&f, 1, 200) > 0) pompe();
    dessine();
    publie_geometrie();
    fprintf(stderr, "CALC_GUI_PRETE pid=%d l=%u h=%u\n", (int)getpid(), vw, vh);
    for (;;) {
        f.revents = 0;
        if (poll(&f, 1, -1) <= 0) continue;
        if (pompe()) dessine();
    }
}
