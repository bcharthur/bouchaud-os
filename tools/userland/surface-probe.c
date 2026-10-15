/*
 * SURFACE-PROBE : banc de dechirure du contrat de surface client
 * (BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1, P10+.3c).
 *
 * Installee en `/bo-navigateur` par `tools/ci/run_banc_dechirure.sh`, elle est
 * lancee par le bureau comme le navigateur (`BO_AUTOSTART_BROWSER=1`) et parle
 * le protocole GUI : Hello, une seconde d'installation (le bureau doit avoir
 * lu la poignee de main et la premiere trame avant la mesure : sans elle, un
 * client rapide finissait ses trames avant la premiere composition), puis des
 * trames pendant BO_SONDE_SURFACE_DUREE_MS (6000 par defaut), chacune d'UNE
 * SEULE COULEUR sur toute la surface, puis elle sort.
 * Le compositeur, sous `gui-surface mesure on`, compte les compositions qui
 * ont lu une trame NON uniforme -- une dechirure -- et le publie a la mort du
 * client (`GUI_SURFACE_BILAN`).
 *
 *   BO_SONDE_SURFACE=v1   le contrat historique : un tampon, on reecrit des
 *                         que la trame est annoncee (rien ne dit quand le
 *                         compositeur a fini de lire) ;
 *   BO_SONDE_SURFACE=v2   deux tampons si le bureau les offre
 *                         (BO_SURFACE_TAMPONS=2) : on n'ecrit que dans un
 *                         tampon libre, rendu par `Liberation`.
 *
 * La regle du contrat est celle du noyau (`Tampons::negocie`) : deux tampons
 * si et seulement si le bureau les offre ET la sonde les annonce. La sonde la
 * calcule elle-meme, sans attendre d'accuse.
 *
 * Sortie : une ligne `SURFACE_SONDE ...` ; code 0, ou 1 sur un defaut (pas de
 * surface, liberation attendue plus de 5 s, ecriture refusee).
 */
#define _GNU_SOURCE
#include <errno.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#define MAGIC 0x55474F42u
#define VERSION 1
#define ENTETE 16
#define HELLO 1
#define FRAME_READY 6
#define LIBERATION 0x108
#define FENETRE 1
#define CAPACITE_DEUX_TAMPONS 1u
#define ATTENTE_MAX_US 5000000ll

static int gui = -1;
static unsigned char recu[65536];
static size_t recu_n = 0;
static int libres[2] = {1, 1};
static long liberations = 0, liberations_etrangeres = 0;

static long long maintenant_us(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000000ll + t.tv_nsec / 1000;
}

static long env_long(const char *nom, long defaut)
{
    const char *v = getenv(nom);
    return v && *v ? strtol(v, NULL, 10) : defaut;
}

static void put32(unsigned char *o, uint32_t v)
{
    o[0] = v; o[1] = v >> 8; o[2] = v >> 16; o[3] = v >> 24;
}

static uint32_t lit32(const unsigned char *o)
{
    return o[0] | (o[1] << 8) | (o[2] << 16) | ((uint32_t)o[3] << 24);
}

/* Un message d'un bloc (en-tete et charge ensemble), comme le chrome. */
static int envoie(uint16_t genre, const unsigned char *charge, uint32_t taille, uint32_t serie, long *refus)
{
    unsigned char m[ENTETE + 64];
    put32(m, MAGIC);
    m[4] = VERSION; m[5] = 0;
    m[6] = genre; m[7] = genre >> 8;
    put32(m + 8, taille);
    put32(m + 12, serie);
    memcpy(m + ENTETE, charge, taille);
    long long debut = maintenant_us();
    for (;;) {
        ssize_t n = write(gui, m, ENTETE + taille);
        if (n == (ssize_t)(ENTETE + taille))
            return 0;
        if (n < 0 && (errno == EAGAIN || errno == EINTR)) {
            ++*refus;
            if (maintenant_us() - debut > ATTENTE_MAX_US)
                return -1;
            struct pollfd p = {gui, POLLOUT, 0};
            poll(&p, 1, 2);
            continue;
        }
        return -1;
    }
}

/* Lit ce qui est arrive (attente au plus `delai_ms`) ; rend le nombre de
 * liberations traitees. */
static int pompe(int delai_ms)
{
    struct pollfd p = {gui, POLLIN, 0};
    if (poll(&p, 1, delai_ms) <= 0 || !(p.revents & POLLIN))
        return 0;
    ssize_t n = read(gui, recu + recu_n, sizeof recu - recu_n);
    if (n <= 0)
        return 0;
    recu_n += (size_t)n;
    int vues = 0;
    size_t i = 0;
    while (recu_n - i >= ENTETE) {
        uint32_t taille = lit32(recu + i + 8);
        if (lit32(recu + i) != MAGIC || taille > 4096) {
            recu_n = 0; /* flux casse : on ne devine pas une resynchronisation */
            return vues;
        }
        if (recu_n - i < ENTETE + taille)
            break;
        uint16_t genre = recu[i + 6] | (recu[i + 7] << 8);
        if (genre == LIBERATION && taille >= 8) {
            uint32_t t = lit32(recu + i + ENTETE + 4);
            if (t < 2 && !libres[t]) {
                libres[t] = 1;
                ++liberations;
                ++vues;
            } else {
                ++liberations_etrangeres;
            }
        }
        i += ENTETE + taille;
    }
    memmove(recu, recu + i, recu_n - i);
    recu_n -= i;
    return vues;
}

static uint32_t couleur(long k, uint32_t precedente)
{
    uint32_t c = ((uint32_t)(k + 1) * 0x9E3779B1u >> 8) & 0x00FFFFFFu;
    if (c == precedente)
        c ^= 0x000101u;
    return c;
}

int main(void)
{
    gui = (int)env_long("BO_GUI_FD", -1);
    int fd_surface = (int)env_long("BO_SURFACE_FD", -1);
    long largeur = env_long("BO_SURFACE_WIDTH", 0);
    long hauteur = env_long("BO_SURFACE_HEIGHT", 0);
    long pas = env_long("BO_SURFACE_STRIDE", 0);
    long offerts = env_long("BO_SURFACE_TAMPONS", 1);
    long duree_ms = env_long("BO_SONDE_SURFACE_DUREE_MS", 6000);
    const char *mode = getenv("BO_SONDE_SURFACE");
    int v2_voulu = mode && strcmp(mode, "v2") == 0;
    if (gui < 0 || fd_surface < 0 || largeur <= 0 || hauteur <= 0 || pas < largeur * 4 || offerts < 1 || offerts > 2) {
        printf("SURFACE_SONDE defaut=environnement gui=%d surface=%d %ldx%ld pas=%ld tampons=%ld\n",
               gui, fd_surface, largeur, hauteur, pas, offerts);
        return 1;
    }
    /* La regle du noyau (`Tampons::negocie`). */
    int tampons = (v2_voulu && offerts >= 2) ? 2 : 1;
    size_t octets_tampon = (size_t)pas * (size_t)hauteur;
    unsigned char *base = mmap(NULL, octets_tampon * (size_t)offerts, PROT_READ | PROT_WRITE, MAP_SHARED, fd_surface, 0);
    if (base == MAP_FAILED) {
        printf("SURFACE_SONDE defaut=mmap errno=%d octets=%zu\n", errno, octets_tampon * (size_t)offerts);
        return 1;
    }

    long refus = 0;
    uint32_t serie = 0;
    unsigned char hello[12];
    put32(hello, 1);
    put32(hello + 4, (uint32_t)getpid());
    put32(hello + 8, v2_voulu ? CAPACITE_DEUX_TAMPONS : 0);
    if (envoie(HELLO, hello, sizeof hello, ++serie, &refus) != 0) {
        printf("SURFACE_SONDE defaut=hello errno=%d\n", errno);
        return 1;
    }
    printf("SURFACE_SONDE_DEBUT mode=%s offerts=%ld tampons=%d surface=%ldx%ld duree_ms=%ld\n",
           v2_voulu ? "v2" : "v1", offerts, tampons, largeur, hauteur, duree_ms);
    fflush(stdout);
    /* Installation : le bureau lit Hello et passe la fenetre en `Actif`. */
    for (int i = 0; i < 100; ++i)
        pompe(10);

    long long debut = maintenant_us(), attente_max = 0, attente_totale = 0;
    long attentes = 0;
    uint32_t precedente = 0xFFFFFFFFu;
    int defaut = 0;
    long k;
    for (k = 0; maintenant_us() - debut < duree_ms * 1000ll; ++k) {
        int b = 0;
        if (tampons == 2) {
            long long t0 = maintenant_us();
            while (!libres[0] && !libres[1]) {
                if (maintenant_us() - t0 > ATTENTE_MAX_US) {
                    defaut = 1;
                    break;
                }
                pompe(5);
            }
            if (defaut)
                break;
            long long attendu = maintenant_us() - t0;
            if (attendu > 0) {
                ++attentes;
                attente_totale += attendu;
                if (attendu > attente_max)
                    attente_max = attendu;
            }
            b = libres[0] ? 0 : 1;
            libres[b] = 0;
        }
        uint32_t c = couleur(k, precedente);
        precedente = c;
        unsigned char *t = base + (size_t)b * octets_tampon;
        for (long y = 0; y < hauteur; ++y) {
            uint32_t *ligne = (uint32_t *)(t + (size_t)y * (size_t)pas);
            for (long x = 0; x < largeur; ++x)
                ligne[x] = c;
        }
        unsigned char trame[24];
        put32(trame, FENETRE);
        put32(trame + 4, (uint32_t)b);
        put32(trame + 8, 0);
        put32(trame + 12, 0);
        put32(trame + 16, (uint32_t)largeur);
        put32(trame + 20, (uint32_t)hauteur);
        if (envoie(FRAME_READY, trame, sizeof trame, ++serie, &refus) != 0) {
            defaut = 2;
            break;
        }
        pompe(0);
    }
    long long duree = maintenant_us() - debut;
    /* Laisser le bureau composer la derniere trame avant de partir. */
    for (int i = 0; i < 40; ++i)
        pompe(10);
    printf("SURFACE_SONDE mode=%s tampons=%d trames=%ld duree_ms=%lld trames_par_s=%lld liberations=%ld "
           "liberations_etrangeres=%ld attentes=%ld attente_moy_us=%lld attente_max_us=%lld ecritures_refusees=%ld defaut=%d\n",
           v2_voulu ? "v2" : "v1", tampons, k, duree / 1000, duree > 0 ? (long long)k * 1000000ll / duree : 0,
           liberations, liberations_etrangeres, attentes, attentes ? attente_totale / attentes : 0, attente_max, refus, defaut);
    fflush(stdout);
    return defaut ? 1 : 0;
}
