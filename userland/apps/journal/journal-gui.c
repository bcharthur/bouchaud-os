/*
 * JOURNAL-GUI -- le Journal d'Atelier en APPLICATION ring 3
 * (BOUCHAUD_JOURNAL_RING3_V1).
 *
 * Remplace `src/gui/apps/journal.rs`, qui capturait, filtrait et dessinait la
 * trace du noyau DANS le noyau, sous le compositeur. Le noyau ne fait plus que
 * PUBLIER : `/proc/bouchaud/journal` (l'anneau de trace au moment de
 * l'ouverture, reserve a la capacite DEBUG) et `/proc/bouchaud/journal_octets`
 * (« total capacite »). Tout le reste est ici : decoupe en lignes, filtres,
 * couleurs, defilement.
 *
 * Meme fenetre au pixel pres que l'ancienne : zone 758x426 (fenetre 760x460),
 * texte a (2,1), DejaVu Sans 13 px de l'atlas d'Atelier, interligne 16,
 * troncature a « largeur / 7 » caracteres, memes couleurs, memes filtres.
 *
 * Clavier : F filtre suivant, R recapture, fleches, Page haut/bas, Debut, Fin.
 * Molette : trois lignes par cran.
 *
 * Journal (sortie d'erreur, lue par les bancs) :
 *   JOURNAL_GUI_PRETE pid= acces= lignes= octets= police=
 *   JOURNAL_GUI_FILTRE nom= retenues=
 *   JOURNAL_GUI_RECAPTURE lignes= octets=
 *   JOURNAL_GUI_FERMEE
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include "atelier.h"

#define BO_MAGIC 0x55474F42u
#define BO_VERSION 1u
#define BO_GUI_HELLO 1u
#define BO_GUI_TITLE 3u
#define BO_GUI_FRAME 6u
#define BO_GUI_CONFIGURE 0x101u
#define BO_GUI_KEY 0x103u
#define BO_GUI_WHEEL 0x105u
#define BO_GUI_CLOSE 0x106u

enum { T_CAR = 0, T_ENTREE, T_RETOUR, T_TAB, T_HAUT, T_BAS, T_GAUCHE, T_DROITE,
       T_ECHAP, T_DEBUT, T_FIN, T_PAGE_HAUT, T_PAGE_BAS, T_SUPPR, T_INSERER, T_FONCTION };

/* La fenetre de l'ancienne application : 760x460, zone utile 758x426. */
#define ZONE_L 758u
#define ZONE_H 426u
#define FOND 0x171b21u          /* theme::COLOR_SURFACE */
#define LIGNE 16
#define PAGE 20

static int gui_fd = -1;
static uint32_t seq = 1;
static uint32_t *surf;
static uint32_t aw, ah, stride, vw, vh;
static struct atelier_police p13;

enum filtre { TOUT, AMORCAGE, USB, STOCKAGE, RESEAU, FAUTES, N_FILTRES };
static const char *const NOM_FILTRE[N_FILTRES] = {
    "TOUT", "AMORCAGE", "USB/HID", "STOCKAGE", "RESEAU", "FAUTES" };

/* Une ligne : le texte et sa forme en majuscules, repliee UNE fois a la
 * capture (le filtre et les couleurs la relisent a chaque trame). */
struct ligne { char *texte, *maj; size_t n; };
static struct ligne *lignes;
static size_t nb_lignes, cap_lignes;
static char *brut;
static size_t octets_total;
static int acces = 1;
static enum filtre filtre = TOUT;
static size_t scroll;
static int suit_la_fin = 1;
#define FIN_SENTINELLE ((size_t)-1)

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

/* --- filtre -------------------------------------------------------------- */

static int a(const char *maj, const char *motif) { return strstr(maj, motif) != NULL; }

static int retient(enum filtre f, const char *m)
{
    switch (f) {
    case TOUT: return 1;
    case AMORCAGE: return a(m, "BOUCHAUD_") || a(m, "[STAGE2]");
    case USB: return a(m, "XHCI") || a(m, "USB") || a(m, "HID") || a(m, "CLAVIER") || a(m, "SOURIS");
    case STOCKAGE: return a(m, "NVME") || a(m, "GPT") || a(m, "BLOC") || a(m, "PERSIST") || a(m, "INSTALL");
    case RESEAU: return a(m, "NET") || a(m, "DHCP") || a(m, "E1000") || a(m, "RTL") || a(m, "ETH0") || a(m, "DNS");
    case FAUTES: return a(m, "FAULT") || a(m, "PANIC") || a(m, "ERREUR") || a(m, "ERROR")
                     || a(m, "ECHEC") || a(m, "FAIL") || a(m, "HORS_SERVICE");
    default: return 1;
    }
}

static size_t retenues(void)
{
    size_t n = 0;
    for (size_t i = 0; i < nb_lignes; i++) n += retient(filtre, lignes[i].maj);
    return n;
}

static uint32_t couleur(const char *m)
{
    if (a(m, "FAULT") || a(m, "PANIC") || a(m, "_FAIL") || a(m, "HORS_SERVICE")) return 0xff6b6b;
    if (a(m, "_OK") || a(m, "_GREEN") || a(m, "_READY") || a(m, "PRET")) return 0x5bda8b;
    if (a(m, "BOUCHAUD_")) return 0xeff3f8;
    return 0x9da8b8;
}

/* --- capture ------------------------------------------------------------- */

static void libere(void)
{
    for (size_t i = 0; i < nb_lignes; i++) free(lignes[i].maj);
    nb_lignes = 0;
    free(brut);
    brut = NULL;
}

static void ajoute(char *debut, size_t n)
{
    if (nb_lignes == cap_lignes) {
        size_t c = cap_lignes ? cap_lignes * 2 : 1024;
        struct ligne *l = realloc(lignes, c * sizeof *l);
        if (!l) return;
        lignes = l;
        cap_lignes = c;
    }
    char *maj = malloc(n + 1);
    if (!maj) return;
    for (size_t i = 0; i < n; i++)
        maj[i] = (debut[i] >= 'a' && debut[i] <= 'z') ? (char)(debut[i] - 32) : debut[i];
    maj[n] = 0;
    debut[n] = 0;
    lignes[nb_lignes++] = (struct ligne){ debut, maj, n };
}

/* Reprend la trace telle qu'elle est maintenant. */
static void capture(void)
{
    libere();
    size_t capacite = 0;
    octets_total = 0;
    FILE *o = fopen("/proc/bouchaud/journal_octets", "r");
    if (o) {
        if (fscanf(o, "%zu %zu", &octets_total, &capacite) != 2) octets_total = capacite = 0;
        fclose(o);
    }
    int fd = open("/proc/bouchaud/journal", O_RDONLY);
    if (fd < 0) {
        acces = 0;
        return;
    }
    acces = 1;
    size_t taille = 0, place = 1 << 20;
    brut = malloc(place + 1);
    ssize_t r;
    while (brut && (r = read(fd, brut + taille, place - taille)) > 0) {
        taille += (size_t)r;
        if (taille == place) {
            char *b = realloc(brut, place * 2 + 1);
            if (!b) break;
            brut = b;
            place *= 2;
        }
    }
    close(fd);
    if (!brut) return;
    /* Meme normalisation que l'ancienne capture : \r retire, \t en espace,
     * tout octet hors ASCII imprimable en « ? ». En place. Les sequences de
     * couleur ANSI de la console serie (ESC [ ... m) sont retirees : l'ancien
     * Journal les affichait en « ?[90m » devant chaque ligne. */
    size_t e = 0, debut = 0;
    for (size_t i = 0; i < taille; i++) {
        unsigned char c = (unsigned char)brut[i];
        if (c == 0x1b && i + 1 < taille && brut[i + 1] == '[') {
            i += 2;
            while (i < taille && !((unsigned char)brut[i] >= 0x40 && (unsigned char)brut[i] <= 0x7e)
                   && brut[i] != '\n')
                i++;
            if (i < taille && brut[i] == '\n') i--;
            continue;
        }
        if (c == '\n') {
            ajoute(brut + debut, e - debut);
            debut = ++e;
            continue;
        }
        if (c == '\r') continue;
        brut[e++] = c == '\t' ? ' ' : (c >= 0x20 && c <= 0x7e) ? (char)c : '?';
    }
    if (e > debut) ajoute(brut + debut, e - debut);
    /* La PREMIERE ligne d'un anneau qui a deja tourne est presque toujours
     * tronquee : la montrer ferait croire a un message coupe par le noyau. */
    if (nb_lignes > 1 && capacite && octets_total > capacite) {
        free(lignes[0].maj);
        memmove(lignes, lignes + 1, (nb_lignes - 1) * sizeof *lignes);
        nb_lignes--;
    }
    scroll = suit_la_fin ? FIN_SENTINELLE : 0;
}

/* --- dessin -------------------------------------------------------------- */

static void boite(uint32_t x, uint32_t y, uint32_t l, uint32_t h, uint32_t c)
{
    for (uint32_t j = y; j < y + h && j < vh; j++)
        for (uint32_t i = x; i < x + l && i < vw; i++) surf[j * (stride / 4) + i] = c;
}

/* `clip(s, cols)` de l'ancienne application : au plus `cols` caracteres. */
static void ecrit(int x, int y, const char *s, size_t cols, uint32_t c, int gras)
{
    size_t n = strlen(s);
    if (n > cols) n = cols;
    atelier_texte_n(&p13, surf, stride, vw, vh, x, y, s, n, c, gras, (int)vw);
}

static size_t visibles(void)
{
    uint32_t bh = vh > 2 ? vh - 2 : 0;
    size_t v = bh / LIGNE;
    return v > 3 ? v - 3 : 1;
}

static void dessine(void)
{
    boite(0, 0, vw, vh, FOND);
    uint32_t bw = vw > 4 ? vw - 4 : 0;
    size_t cols = bw / 7;
    size_t vis = visibles(), ret = retenues();
    size_t plafond = ret > vis ? ret - vis : 0;
    size_t depart = scroll < plafond ? scroll : plafond;
    int x = 2, y = 1;
    char tete[160];
    snprintf(tete, sizeof tete, "JOURNAL NOYAU  filtre:%s  %zu/%zu lignes  %zu o",
             NOM_FILTRE[filtre], ret, nb_lignes, octets_total);
    ecrit(x, y, tete, cols, 0xf5be4a, 1);
    y += LIGNE;
    ecrit(x, y, "F filtre   R recapture   fleches/PgUp/PgDn/Origine/Fin defilent", cols, 0x67d5e8, 0);
    y += LIGNE;
    if (!acces) {
        ecrit(x, y, "(trace du noyau refusee : capacite DEBUG requise)", cols, 0xff6b6b, 0);
    } else {
        size_t rang = 0, dessinees = 0;
        for (size_t i = 0; i < nb_lignes && dessinees < vis; i++) {
            if (!retient(filtre, lignes[i].maj)) continue;
            if (rang++ < depart) continue;
            ecrit(x, y, lignes[i].texte, cols, couleur(lignes[i].maj), 0);
            y += LIGNE;
            dessinees++;
        }
        if (dessinees == 0) ecrit(x, y, "(aucune ligne ne passe ce filtre)", cols, 0x9da8b8, 0);
    }
    uint8_t t[24];
    put32(t, 1); put32(t + 4, 0); put32(t + 8, 0); put32(t + 12, 0);
    put32(t + 16, vw); put32(t + 20, vh);
    envoie(BO_GUI_FRAME, t, 24);
}

/* --- entrees ------------------------------------------------------------- */

/* Defile en restant dans ce que le filtre retient ; resout la sentinelle
 * « la fin » AVANT d'ajouter le delta (sinon la premiere fleche vers le bas
 * renverrait au debut). */
static void deplace(long delta, size_t vis)
{
    size_t ret = retenues();
    size_t plafond = ret > vis ? ret - vis : 0;
    long courant = (long)(scroll < plafond ? scroll : plafond);
    long nouveau = courant + delta;
    if (nouveau < 0) nouveau = 0;
    if (nouveau > (long)plafond) nouveau = (long)plafond;
    scroll = (size_t)nouveau;
    suit_la_fin = scroll >= plafond;
}

static void touche(uint32_t code, uint32_t car)
{
    if (code == T_CAR && (car == 'f' || car == 'F')) {
        filtre = (enum filtre)((filtre + 1) % N_FILTRES);
        scroll = 0;
        fprintf(stderr, "JOURNAL_GUI_FILTRE nom=%s retenues=%zu\n", NOM_FILTRE[filtre], retenues());
    } else if (code == T_CAR && (car == 'r' || car == 'R')) {
        capture();
        fprintf(stderr, "JOURNAL_GUI_RECAPTURE lignes=%zu octets=%zu\n", nb_lignes, octets_total);
    } else if (code == T_HAUT) deplace(-1, PAGE);
    else if (code == T_BAS) deplace(1, PAGE);
    else if (code == T_PAGE_HAUT) deplace(-PAGE, PAGE);
    else if (code == T_PAGE_BAS) deplace(PAGE, PAGE);
    else if (code == T_DEBUT) { scroll = 0; suit_la_fin = 0; }
    else if (code == T_FIN) deplace(1L << 30, PAGE);
}

static int pompe(void)
{
    static uint8_t rx[8192];
    static size_t plein;
    int redessine = 0;
    ssize_t lu = read(gui_fd, rx + plein, sizeof rx - plein);
    if (lu <= 0) _exit(0);
    plein += (size_t)lu;
    size_t o = 0;
    for (; o + 16 <= plein;) {
        uint32_t len = get32(rx + o + 8);
        uint16_t genre = rx[o + 6] | (rx[o + 7] << 8);
        const uint8_t *c = rx + o + 16;
        if (len > sizeof rx - 16) { plein = 0; return redessine; }
        if (o + 16 + len > plein) break;
        if (genre == BO_GUI_CLOSE) {
            fprintf(stderr, "JOURNAL_GUI_FERMEE\n");
            _exit(0);
        }
        if (genre == BO_GUI_KEY && len >= 20 && get32(c + 16)) {
            touche(get32(c + 4), get32(c + 12));
            redessine = 1;
        }
        if (genre == BO_GUI_WHEEL && len >= 16) {
            /* Positif vers le haut, 120 par cran (protocole) ; trois lignes
             * par cran, comme `journal::on_wheel`. */
            int32_t d = (int32_t)get32(c + 4);
            long crans = d / 120 ? d / 120 : (d > 0) - (d < 0);
            deplace(-crans * 3, PAGE);
            redessine = 1;
        }
        if (genre == BO_GUI_CONFIGURE && len >= 16) {
            uint32_t w = get32(c + 4), h = get32(c + 8);
            if (w && h && w <= aw && h <= ah && (w != vw || h != vh)) { vw = w; vh = h; redessine = 1; }
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
    if (!gui_fd || !aw || !ah || stride < aw * 4) { fprintf(stderr, "JOURNAL_GUI environnement absent\n"); return 1; }
    surf = mmap(0, (size_t)stride * ah, PROT_READ | PROT_WRITE, MAP_SHARED, sfd, 0);
    if (surf == MAP_FAILED) { perror("JOURNAL_GUI mmap"); return 1; }
    int police = atelier_police_charge(&p13, 13) == 0;
    capture();
    vw = aw < ZONE_L ? aw : ZONE_L; vh = ah < ZONE_H ? ah : ZONE_H;
    uint8_t hello[8]; put32(hello, BO_VERSION); put32(hello + 4, (uint32_t)getpid());
    envoie(BO_GUI_HELLO, hello, 8);
    const char *titre = "Journal \xe2\x80\x94 TOUT";
    envoie(BO_GUI_TITLE, titre, (uint32_t)strlen(titre));
    struct pollfd f = { gui_fd, POLLIN, 0 };
    if (poll(&f, 1, 200) > 0) pompe();
    dessine();
    fprintf(stderr, "JOURNAL_GUI_PRETE pid=%d acces=%d lignes=%zu octets=%zu police=%d\n",
            (int)getpid(), acces, nb_lignes, octets_total, police);
    for (;;) {
        f.revents = 0;
        if (poll(&f, 1, -1) <= 0) continue;
        if (pompe()) dessine();
    }
}
