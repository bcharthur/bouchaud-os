/*
 * NOTES-GUI -- Bouchaud Notes, en APPLICATION ring 3 (BOUCHAUD_NOTES_RING3_V1).
 *
 * Remplace `src/gui/apps/notes.rs`, qui editait, dessinait et ecrivait le
 * disque DANS le noyau. Client ordinaire du protocole GUI (comme la
 * Calculatrice) : surface partagee + canal, entrees recues du gestionnaire de
 * fenetres. Le texte s'affiche avec la police d'Atelier publiee par le noyau
 * (`userland/libs/police/atelier.h`) : meme rendu que le bureau.
 *
 * Fichier : $BO_NOTES_FICHIER (ouvert depuis Fichiers), sinon
 * /persist/bouchaud-notes.txt si la zone persistante existe, sinon
 * ./bouchaud-notes.txt (le dossier personnel). Enregistrer = ecrire puis
 * `fsync` : pour un fichier de /persist, c'est la transaction disque.
 *
 * Clavier : caracteres, Entree, Tab, Retour arriere, Suppr, fleches, Debut,
 * Fin, Page haut/bas ; F2 enregistrer, F3 recharger (seulement si propre).
 * Souris : boutons « F2 Sauver » et « F3 Recharger » ; clic dans le texte :
 * place le curseur.
 *
 * Journal (sortie d'erreur, lue par les bancs) :
 *   NOTES_GUI_PRETE pid= fichier= durable= octets= police=
 *   NOTES_GUI_SAUVE fichier= octets= durable= fsync=
 *   NOTES_GUI_RECHARGE octets=
 *   NOTES_GUI_TOUCHE code= unicode=
 *   NOTES_GUI_FERMEE
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
#include <sys/stat.h>
#include <unistd.h>

#include "atelier.h"

#define BO_MAGIC 0x55474F42u
#define BO_VERSION 1u
#define BO_GUI_HELLO 1u
#define BO_GUI_TITLE 3u
#define BO_GUI_FRAME 6u
#define BO_GUI_CONFIGURE 0x101u
#define BO_GUI_KEY 0x103u
#define BO_GUI_POINTER 0x104u
#define BO_GUI_CLOSE 0x106u

/* Codes logiques du protocole (window_manager::touche). */
enum { T_CAR = 0, T_ENTREE, T_RETOUR, T_TAB, T_HAUT, T_BAS, T_GAUCHE, T_DROITE,
       T_ECHAP, T_DEBUT, T_FIN, T_PAGE_HAUT, T_PAGE_BAS, T_SUPPR, T_INSERER, T_FONCTION };

#define LIMITE (256 * 1024)
#define LIGNES_VISIBLES 17u
#define HAUTEUR_LIGNE 20u

static int gui_fd = -1;
static uint32_t seq = 1;
static uint32_t *surf;
static uint32_t aw, ah, stride, vw, vh;
static struct atelier_police p12, p13, p14;

static char chemin[512];
static int durable;
static char *texte;
static size_t taille, curseur, defilement;
static int sale;
static char statut[640];

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

/* --- fichier ------------------------------------------------------------- */

static const char *nom_fichier(void)
{
    const char *n = strrchr(chemin, '/');
    return n ? n + 1 : chemin;
}

static void choisit_chemin(void)
{
    const char *f = getenv("BO_NOTES_FICHIER");
    struct stat st;
    if (f && f[0] == '/') snprintf(chemin, sizeof chemin, "%s", f);
    else if (stat("/persist", &st) == 0 && S_ISDIR(st.st_mode))
        snprintf(chemin, sizeof chemin, "/persist/bouchaud-notes.txt");
    else snprintf(chemin, sizeof chemin, "bouchaud-notes.txt");
    durable = strncmp(chemin, "/persist/", 9) == 0;
}

static void recharge(void)
{
    taille = curseur = defilement = 0;
    texte[0] = 0;
    sale = 0;
    int fd = open(chemin, O_RDONLY);
    if (fd < 0) {
        snprintf(statut, sizeof statut, "Nouveau document %s%s", nom_fichier(),
                 durable ? " (persistant)" : "");
        return;
    }
    ssize_t r;
    while (taille < LIMITE && (r = read(fd, texte + taille, LIMITE - taille)) > 0)
        taille += (size_t)r;
    close(fd);
    texte[taille] = 0;
    snprintf(statut, sizeof statut, "%s ouvert%s", nom_fichier(), durable ? " (persistant)" : "");
    fprintf(stderr, "NOTES_GUI_RECHARGE octets=%zu\n", taille);
}

static void sauve(void)
{
    int fd = open(chemin, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) {
        snprintf(statut, sizeof statut, "ECHEC sauvegarde : ouverture (errno %d)", errno);
        return;
    }
    size_t ecrit = 0;
    while (ecrit < taille) {
        ssize_t w = write(fd, texte + ecrit, taille - ecrit);
        if (w <= 0) break;
        ecrit += (size_t)w;
    }
    int f = fsync(fd);
    int c = close(fd);
    if (ecrit != taille || c != 0) {
        snprintf(statut, sizeof statut, "ECHEC sauvegarde : ecriture incomplete");
        return;
    }
    /* Un fichier volatile est bien sauvegarde ; un fichier persistant ne
     * l'est qu'une fois la transaction disque confirmee. */
    sale = durable && f != 0;
    if (durable)
        snprintf(statut, sizeof statut, f == 0 ? "Enregistre sur disque : %s"
                 : "Ecrit en memoire, disque NON confirme : %s", nom_fichier());
    else
        snprintf(statut, sizeof statut, "Enregistre (RAMFS, non persistant) : %s", nom_fichier());
    fprintf(stderr, "NOTES_GUI_SAUVE fichier=%s octets=%zu durable=%d fsync=%d\n",
            chemin, taille, durable, f);
}

/* --- edition (memes regles que notes.rs) ---------------------------------- */

static size_t ligne_du_curseur(void)
{
    size_t r = 0;
    for (size_t i = 0; i < curseur; i++) r += texte[i] == '\n';
    return r;
}

static void suit_defilement(void)
{
    size_t row = ligne_du_curseur();
    if (row < defilement) defilement = row;
    if (row >= defilement + LIGNES_VISIBLES) defilement = row - LIGNES_VISIBLES + 1;
}

static size_t precedent(size_t i) { if (!i) return 0; i--; while (i && (texte[i] & 0xc0) == 0x80) i--; return i; }
static size_t suivant(size_t i) { if (i >= taille) return taille; i++; while (i < taille && (texte[i] & 0xc0) == 0x80) i++; return i; }
static size_t debut_ligne(size_t i) { while (i && texte[i - 1] != '\n') i--; return i; }
static size_t fin_ligne(size_t i) { while (i < taille && texte[i] != '\n') i++; return i; }

static void insere(const char *s, size_t n)
{
    if (taille + n > LIMITE) { snprintf(statut, sizeof statut, "Limite document atteinte"); return; }
    memmove(texte + curseur + n, texte + curseur, taille - curseur + 1);
    memcpy(texte + curseur, s, n);
    taille += n; curseur += n; sale = 1;
    suit_defilement();
}

static void efface(size_t de, size_t a)
{
    if (a <= de) return;
    memmove(texte + de, texte + a, taille - a + 1);
    taille -= a - de; curseur = de; sale = 1;
    suit_defilement();
}

static size_t caracteres(size_t de, size_t a)
{
    size_t n = 0;
    for (size_t i = de; i < a; i++) n += (texte[i] & 0xc0) != 0x80;
    return n;
}

static size_t avance_car(size_t de, size_t lim, size_t n)
{
    while (n-- && de < lim) de = suivant(de) < lim ? suivant(de) : lim;
    return de;
}

static void vertical(int bas)
{
    size_t d = debut_ligne(curseur), col = caracteres(d, curseur);
    if (bas) {
        size_t f = fin_ligne(curseur);
        if (f == taille) return;
        size_t nd = f + 1, nf = fin_ligne(nd);
        curseur = avance_car(nd, nf, col);
    } else {
        if (d == 0) return;
        size_t pf = d - 1, pd = debut_ligne(pf);
        curseur = avance_car(pd, pf, col);
    }
    suit_defilement();
}

static void touche(uint32_t code, uint32_t u)
{
    fprintf(stderr, "NOTES_GUI_TOUCHE code=%u unicode=%u\n", code, u);
    switch (code) {
    case T_FONCTION:
        if (u == 2) sauve();
        else if (u == 3) {
            if (sale) snprintf(statut, sizeof statut, "Modifications non sauvegardees : F2 avant F3");
            else recharge();
        }
        break;
    case T_ENTREE: insere("\n", 1); break;
    case T_TAB: insere("    ", 4); break;
    case T_CAR:
        if (u >= 32 && u != 127 && u < 256) {
            /* Le gestionnaire livre le caractere en Latin-1 ; on stocke UTF-8. */
            char b[2];
            if (u < 0x80) { b[0] = (char)u; insere(b, 1); }
            else { b[0] = (char)(0xc0 | (u >> 6)); b[1] = (char)(0x80 | (u & 0x3f)); insere(b, 2); }
        }
        break;
    case T_RETOUR: if (curseur) efface(precedent(curseur), curseur); break;
    case T_SUPPR: efface(curseur, suivant(curseur)); break;
    case T_GAUCHE: curseur = precedent(curseur); suit_defilement(); break;
    case T_DROITE: curseur = suivant(curseur); suit_defilement(); break;
    case T_DEBUT: curseur = debut_ligne(curseur); suit_defilement(); break;
    case T_FIN: curseur = fin_ligne(curseur); suit_defilement(); break;
    case T_HAUT: vertical(0); break;
    case T_BAS: vertical(1); break;
    case T_PAGE_HAUT: defilement = defilement > LIGNES_VISIBLES ? defilement - LIGNES_VISIBLES : 0; break;
    case T_PAGE_BAS: {
        size_t n = 0;
        for (size_t i = 0; i < taille; i++) n += texte[i] == '\n';
        defilement = defilement + LIGNES_VISIBLES < n ? defilement + LIGNES_VISIBLES : n;
        break;
    }
    default: break;
    }
}

/* --- dessin (copie de notes::draw) ---------------------------------------- */

static void boite(uint32_t x, uint32_t y, uint32_t l, uint32_t h, uint32_t c)
{
    for (uint32_t j = y; j < y + h && j < vh; j++) {
        uint32_t *ligne = (uint32_t *)((uint8_t *)surf + (size_t)j * stride);
        for (uint32_t i = x; i < x + l && i < vw; i++) ligne[i] = c;
    }
}

static void ecrit(const struct atelier_police *p, int x, int y, const char *s, size_t n,
                  uint32_t c, int gras, int x_max)
{
    atelier_texte_n(p, surf, stride, vw, vh, x, y, s, n, c, gras, x_max);
}

static uint32_t zone_max(void) { uint32_t m = vh > 92 ? (vh - 92) / HAUTEUR_LIGNE : 0; return m < LIGNES_VISIBLES ? m : LIGNES_VISIBLES; }

static void dessine(void)
{
    if (vw < 40 || vh < 90) return;
    boite(0, 0, vw, vh, 0x101827);
    boite(0, 0, vw, 46, 0x1c2a3d);
    boite(10, 8, 105, 30, 0x2463b8);
    boite(120, 8, 125, 30, 0x33465e);
    ecrit(&p13, 18, 15, "F2  Sauver", 10, 0xffffff, 1, (int)vw);
    ecrit(&p13, 130, 15, "F3  Recharger", 13, 0xffffff, 1, (int)vw);
    const char *marque = sale ? "NON SAUVE" : durable ? "PERSIST" : "RAMFS";
    ecrit(&p12, 255, 16, marque, strlen(marque), 0xf7c569, 1, (int)vw);

    uint32_t y = 53, max = zone_max();
    size_t debut = 0, row = 0;
    for (;;) {
        size_t fin = fin_ligne(debut);
        if (row >= defilement && row < defilement + max) {
            char num[8];
            snprintf(num, sizeof num, "%03zu", row + 1);
            ecrit(&p12, 12, (int)y, num, strlen(num), 0x71849d, 0, (int)vw);
            int x_max = vw > 20 ? (int)vw - 20 : 0;
            ecrit(&p14, 62, (int)y, texte + debut, fin - debut, 0xe2e8f0, 0, x_max);
            if (curseur >= debut && curseur <= fin) {
                int cx = 62 + atelier_largeur_n(&p14, texte + debut, curseur - debut);
                if (cx < (int)vw - 3) boite((uint32_t)cx, y, 2, 17, 0x67e8f9);
            }
            y += HAUTEUR_LIGNE;
        }
        if (fin >= taille || row >= defilement + max) break;
        debut = fin + 1;
        row++;
    }
    uint32_t pied = vh - 30;
    boite(0, pied, vw, 30, 0x1c2a3d);
    ecrit(&p12, 10, (int)pied + 7, statut, strlen(statut), 0xf7c569, 0, (int)vw - 14);

    uint8_t t[24];
    put32(t, 1); put32(t + 4, 0); put32(t + 8, 0); put32(t + 12, 0);
    put32(t + 16, vw); put32(t + 20, vh);
    envoie(BO_GUI_FRAME, t, 24);
}

/* Clic dans le texte : la ligne sous le pointeur, puis la colonne. */
static void place_curseur(int x, int y)
{
    if (y < 53) return;
    size_t cible = defilement + (size_t)(y - 53) / HAUTEUR_LIGNE;
    if (cible >= defilement + zone_max()) return;
    size_t d = 0;
    for (size_t r = 0; r < cible; r++) {
        size_t f = fin_ligne(d);
        if (f >= taille) return;
        d = f + 1;
    }
    size_t f = fin_ligne(d), i = d;
    while (i < f) {
        size_t n = suivant(i);
        if (62 + atelier_largeur_n(&p14, texte + d, n - d) > x + 3) break;
        i = n;
    }
    curseur = i;
}

static void clic(int x, int y)
{
    if (y >= 8 && y <= 45) {
        if (x >= 10 && x < 115) sauve();
        else if (x >= 120 && x < 245) {
            if (sale) snprintf(statut, sizeof statut, "F2 pour sauvegarder avant de recharger");
            else recharge();
        }
        return;
    }
    place_curseur(x, y);
}

/* --- canal ----------------------------------------------------------------- */

static int pompe(void)
{
    static uint8_t rx[8192];
    static size_t plein;
    static uint32_t boutons_prec;
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
            fprintf(stderr, "NOTES_GUI_FERMEE sale=%d\n", sale);
            _exit(0);
        }
        if (genre == BO_GUI_KEY && len >= 20 && get32(c + 16)) {
            touche(get32(c + 4), get32(c + 12));
            redessine = 1;
        }
        if (genre == BO_GUI_POINTER && len >= 16) {
            int32_t x = (int32_t)get32(c + 4), y = (int32_t)get32(c + 8);
            uint32_t b = get32(c + 12);
            if ((b & 1) && !(boutons_prec & 1) && x >= 0 && y >= 0) { clic(x, y); redessine = 1; }
            boutons_prec = b;
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
    if (!gui_fd || !aw || !ah || stride < aw * 4) { fprintf(stderr, "NOTES_GUI environnement absent\n"); return 1; }
    surf = mmap(0, (size_t)stride * ah, PROT_READ | PROT_WRITE, MAP_SHARED, sfd, 0);
    if (surf == MAP_FAILED) { perror("NOTES_GUI mmap"); return 1; }
    texte = malloc(LIMITE + 1);
    if (!texte) return 1;
    int police = atelier_police_charge(&p12, 12) == 0 && atelier_police_charge(&p13, 13) == 0
              && atelier_police_charge(&p14, 14) == 0;
    choisit_chemin();
    recharge();
    vw = aw < 660 ? aw : 660; vh = ah < 470 ? ah : 470;
    uint8_t hello[8]; put32(hello, BO_VERSION); put32(hello + 4, (uint32_t)getpid());
    envoie(BO_GUI_HELLO, hello, 8);
    char titre[600];
    snprintf(titre, sizeof titre, "Bouchaud Notes - %s", nom_fichier());
    envoie(BO_GUI_TITLE, titre, (uint32_t)strlen(titre));
    struct pollfd f = { gui_fd, POLLIN, 0 };
    if (poll(&f, 1, 200) > 0) pompe();
    dessine();
    fprintf(stderr, "NOTES_GUI_PRETE pid=%d fichier=%s durable=%d octets=%zu police=%d\n",
            (int)getpid(), chemin, durable, taille, police);
    for (;;) {
        f.revents = 0;
        if (poll(&f, 1, -1) <= 0) continue;
        if (pompe()) dessine();
    }
}
