/*
 * `composited` -- le premier tranchant vertical du compositeur ring 3.
 *
 * CE QUE CE PROGRAMME PROUVE
 * ==========================
 *
 * Que le chemin complet d'une trame passe par l'ABI NATIVE de Bouchaud, en
 * ring 3, sans une seule primitive Linux :
 *
 *   1. le compositeur cree la region partagee (deux tampons) et un canal ;
 *   2. le client demande une surface ;
 *   3. le compositeur la lui accorde, et lui transfere la region en
 *      LECTURE-ECRITURE ATTENUEE -- pas de DUP, pas de TRANSFER : le client ne
 *      peut ni multiplier ni repasser la capacite ;
 *   4. le client dessine dans le tampon qu'il POSSEDE ;
 *   5. il annonce `TrameLivree { tampon, degat }` et signale l'evenement ;
 *   6. le compositeur attend sur un waitset, compose, presente ;
 *   7. il RENVOIE le tampon libere -- le pas qu'on oublie, et celui dont
 *      l'absence dechire l'affichage.
 *
 * CE QU'IL NE PROUVE PAS
 * ======================
 *
 * Les deux roles vivent dans le meme processus, relies par un vrai canal
 * natif. L'isolation par processus viendra avec le lancement de service ; ce
 * qui est deja reel ici, c'est le PROTOCOLE, les objets noyau, l'attenuation
 * des droits et la propriete des tampons. Ce sont eux qui decident si un
 * compositeur ring 3 est possible ; le decoupage en processus est ensuite une
 * question de lancement, pas de contrat.
 *
 * Aucune libc n'est liee. Les seuls appels Linux sont `write` sur la sortie
 * serie et `exit_group`, pour rendre un verdict.
 */

typedef unsigned long u64;
typedef unsigned int u32;
typedef long i64;

#include "protocole.h"

#define BO_BASE           0x424f0000UL
#define BO_HANDLE_CLOSE   (BO_BASE + 0x01)
#define BO_HANDLE_DUP     (BO_BASE + 0x02)
#define BO_HANDLE_INFO    (BO_BASE + 0x03)
#define BO_CHANNEL_CREATE (BO_BASE + 0x10)
#define BO_CHANNEL_SEND   (BO_BASE + 0x11)
#define BO_CHANNEL_RECV   (BO_BASE + 0x12)
#define BO_CHANNEL_SEND_ATTENUE (BO_BASE + 0x13)
#define BO_EVENT_CREATE   (BO_BASE + 0x20)
#define BO_EVENT_SIGNAL   (BO_BASE + 0x21)
#define BO_EVENT_RESET    (BO_BASE + 0x22)
#define BO_WAITSET_CREATE (BO_BASE + 0x30)
#define BO_WAITSET_ADD    (BO_BASE + 0x31)
#define BO_WAITSET_POLL   (BO_BASE + 0x33)
#define BO_SHM_CREATE     (BO_BASE + 0x40)
#define BO_SHM_SIZE       (BO_BASE + 0x41)
#define BO_SHM_READ       (BO_BASE + 0x42)
#define BO_SHM_WRITE      (BO_BASE + 0x43)
#define BO_DISPLAY_PRESENT (BO_BASE + 0x80)

#define BO_RIGHT_READ     (1u << 0)
#define BO_RIGHT_WRITE    (1u << 1)
#define BO_RIGHT_MAP      (1u << 3)
#define BO_RIGHT_DUP      (1u << 4)
#define BO_RIGHT_TRANSFER (1u << 5)
#define BO_RIGHT_INSPECT  (1u << 6)
#define BO_E_INVALID_ARGUMENT 2
#define BO_E_ACCESS_DENIED    5

#define BO_SIGNAL_SIGNALED (1u << 2)

struct bo_recv_meta { u64 bytes; u64 handles; };
struct bo_handle_info { u32 kind; u32 rights; u32 signals; u32 reserved; };
struct bo_wait_event { u64 key; u32 signals; u32 reserved; };

static inline i64 sys6(u64 nr, u64 a, u64 b, u64 c, u64 d, u64 e, u64 f)
{
    register u64 r10 __asm__("r10") = d;
    register u64 r8  __asm__("r8")  = e;
    register u64 r9  __asm__("r9")  = f;
    i64 ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory");
    return ret;
}

static void ecris(const char *texte)
{
    unsigned long n = 0;
    while (texte[n]) n++;
    sys6(1, 1, (u64)texte, n, 0, 0, 0);
}

/* Une ligne = UNE ecriture : le compositeur lance par le noyau ecrit sur la
 * meme console que les sondes de l'autorun, en meme temps qu'elles. */
static char ligne_sortie[192];

static void ecris_ligne(const char *a, const char *b)
{
    unsigned long n = 0;
    for (; *a && n < sizeof(ligne_sortie) - 2; a++) ligne_sortie[n++] = *a;
    for (; *b && n < sizeof(ligne_sortie) - 2; b++) ligne_sortie[n++] = *b;
    ligne_sortie[n++] = '\n';
    ligne_sortie[n] = 0;
    ecris(ligne_sortie);
}

static void ecris_nombre(const char *etiquette, i64 valeur)
{
    char tampon[32];
    int i = 30;
    int negatif = valeur < 0;
    u64 reste = negatif ? (u64)(-valeur) : (u64)valeur;
    tampon[31] = 0;
    if (reste == 0) tampon[i--] = '0';
    while (reste) { tampon[i--] = (char)('0' + (reste % 10)); reste /= 10; }
    if (negatif) tampon[i--] = '-';
    ecris_ligne(etiquette, &tampon[i + 1]);
}

static int echecs = 0;

static void exige(int condition, const char *nom)
{
    if (!condition) echecs++;
    ecris_ligne(condition ? "[COMPOSITED] ok   " : "[COMPOSITED] ECHEC ", nom);
}

/* --- Geometrie du tranchant ------------------------------------------------ */

#define LARGEUR   64u
#define HAUTEUR   32u
#define PAS       (LARGEUR * 4u)
#define OCTETS_TAMPON (PAS * HAUTEUR)
#define OCTETS_REGION (OCTETS_TAMPON * COMPOSITED_TAMPONS)

/* Le compositeur, reduit a ce qui decide : la propriete des tampons. */
enum proprietaire { PROP_CLIENT = 0, PROP_COMPOSITEUR = 1, PROP_AFFICHE = 2 };

struct surface {
    u32 id;
    u32 largeur, hauteur, pas;
    enum proprietaire proprietaires[COMPOSITED_TAMPONS];
    int affiche;              /* -1 si aucun */
    u32 degat_x, degat_y, degat_w, degat_h;
    u32 derniere_trame;
};

static u32 pixels_composes = 0;
static u32 trames_presentees = 0;

/* Rogne un degat a la surface : un client peut se tromper, ou mentir. */
static void accumule_degat(struct surface *s, u32 x, u32 y, u32 w, u32 h)
{
    if (x >= s->largeur || y >= s->hauteur || w == 0 || h == 0) return;
    if (x + w > s->largeur) w = s->largeur - x;
    if (y + h > s->hauteur) h = s->hauteur - y;
    if (s->degat_w == 0 || s->degat_h == 0) {
        s->degat_x = x; s->degat_y = y; s->degat_w = w; s->degat_h = h;
        return;
    }
    u32 x0 = s->degat_x < x ? s->degat_x : x;
    u32 y0 = s->degat_y < y ? s->degat_y : y;
    u32 x1 = (s->degat_x + s->degat_w) > (x + w) ? (s->degat_x + s->degat_w) : (x + w);
    u32 y1 = (s->degat_y + s->degat_h) > (y + h) ? (s->degat_y + s->degat_h) : (y + h);
    s->degat_x = x0; s->degat_y = y0; s->degat_w = x1 - x0; s->degat_h = y1 - y0;
}


/* --- BOUCHAUD_COMPOSITORD_PRESENTATION_V1 : poser une surface a l'ecran ----
 *
 * Lance par le NOYAU sous le nom `/bin/composited` (seule origine de la
 * capacite d'affichage), le service recoit `BO_DISPLAY=<handle>` dans son
 * environnement. Il remplit une surface 320x200 avec une MIRE (chaque pixel
 * encode sa position, la meme que `tools/userland/mire-gpu.c`), la fait
 * poser en (100,100) par `bo_display_present`, et compare l'empreinte des
 * pixels RELUS a l'ecran par le noyau avec la sienne. Le banc capture ensuite
 * l'ecran de QEMU et decode la mire au pixel.
 *
 * Sans capacite (lance par l'autorun), seul le refus est verifie.
 */

#define MIRE_L 320u
#define MIRE_H 200u
#define MIRE_X 100u
#define MIRE_Y 100u

static u32 pixel_mire(u32 x, u32 y)
{
    u32 r = y & 0xffu, g = x & 0xffu, b = 0x80u | ((y >> 8) << 3) | (x >> 8);
    return (r << 16) | (g << 8) | b;
}

/* FNV-1a 64 bits sur les trois octets RVB, tronquee a 62 bits : la meme que
 * `affichage_contrat::empreinte` (test hote `test_affichage_contrat.rs`). */
static u64 empreinte_fnv(u32 l, u32 h)
{
    u64 e = 0xcbf29ce484222325UL;
    for (u32 y = 0; y < h; y++)
        for (u32 x = 0; x < l; x++) {
            u32 v = pixel_mire(x, y) & 0xffffffu;
            for (u32 k = 0; k < 4; k++) {
                e ^= (v >> (8 * k)) & 0xffu;
                e *= 0x100000001b3UL;
            }
        }
    return e & 0x3fffffffffffffffUL;
}

struct demande_presentation {
    u32 largeur, hauteur, x, y, l, h, dest_x, dest_y;
};

static i64 presente(i64 affichage, i64 region, u32 x, u32 y, u32 l, u32 h, u32 dx, u32 dy)
{
    struct demande_presentation d = { MIRE_L, MIRE_H, x, y, l, h, dx, dy };
    return sys6(BO_DISPLAY_PRESENT, (u64)affichage, (u64)region, (u64)&d, 0, 0, 0);
}

static u32 ligne_mire[MIRE_L];

static i64 region_mire(void)
{
    i64 region = sys6(BO_SHM_CREATE, MIRE_L * MIRE_H * 4u, 0, 0, 0, 0, 0);
    if (region < 0) return region;
    for (u32 y = 0; y < MIRE_H; y++) {
        for (u32 x = 0; x < MIRE_L; x++) ligne_mire[x] = pixel_mire(x, y);
        if (sys6(BO_SHM_WRITE, (u64)region, (u64)y * MIRE_L * 4u, (u64)ligne_mire, MIRE_L * 4u, 0, 0) < 0)
            return -1;
    }
    return region;
}

static int presentation(i64 affichage)
{
    i64 region = region_mire();
    exige(region > 0, "presentation : region de la mire creee et remplie");
    if (region <= 0) return 1;

    if (affichage < 0) {
        /* Sans capacite : une region n'est pas une capacite d'affichage. */
        i64 r = presente(region, region, 0, 0, MIRE_L, MIRE_H, MIRE_X, MIRE_Y);
        ecris_nombre("[COMPOSITED] present_sans_capacite=", r);
        exige(r == -BO_E_ACCESS_DENIED, "presentation refusee sans capacite affichage");
        sys6(BO_HANDLE_CLOSE, (u64)region, 0, 0, 0, 0, 0);
        return 0;
    }

    /* Refus AVEC la capacite : bornes et droits. */
    i64 r = presente(affichage, region, 1, 0, MIRE_L, MIRE_H, MIRE_X, MIRE_Y);
    exige(r == -BO_E_INVALID_ARGUMENT, "rectangle hors de la surface refuse");
    r = presente(affichage, region, 0, 0, MIRE_L, MIRE_H, 0xfffffff0u, MIRE_Y);
    exige(r == -BO_E_INVALID_ARGUMENT, "destination hors ecran (debordement u32) refusee");
    i64 ecriture_seule = sys6(BO_HANDLE_DUP, (u64)region, BO_RIGHT_WRITE, 0, 0, 0, 0);
    exige(ecriture_seule > 0, "region dupliquee en ecriture seule");
    r = presente(affichage, ecriture_seule, 0, 0, MIRE_L, MIRE_H, MIRE_X, MIRE_Y);
    exige(r == -BO_E_ACCESS_DENIED, "region sans droit de lecture refusee");
    r = presente(affichage, affichage, 0, 0, MIRE_L, MIRE_H, MIRE_X, MIRE_Y);
    exige(r == -BO_E_ACCESS_DENIED, "la capacite ne sert pas de region");
    i64 copie = sys6(BO_HANDLE_DUP, (u64)affichage, BO_RIGHT_INSPECT, 0, 0, 0, 0);
    exige(copie < 0, "la capacite affichage ne se duplique pas");

    /* La presentation reelle, verifiee par relecture. */
    u64 attendue = empreinte_fnv(MIRE_L, MIRE_H);
    r = presente(affichage, region, 0, 0, MIRE_L, MIRE_H, MIRE_X, MIRE_Y);
    ecris_nombre("[COMPOSITED] empreinte_attendue=", (i64)attendue);
    ecris_nombre("[COMPOSITED] empreinte_relue=", r);
    exige(r >= 0 && (u64)r == attendue, "mire posee sur ecran, relue au pixel par le noyau");

    sys6(BO_HANDLE_CLOSE, (u64)ecriture_seule, 0, 0, 0, 0, 0);
    sys6(BO_HANDLE_CLOSE, (u64)region, 0, 0, 0, 0, 0);
    if (echecs == 0) {
        ecris("COMPOSITED_PRESENTATION_OK rect=320x200@100,100\n");
        return 0;
    }
    ecris("COMPOSITED_PRESENTATION_FAIL\n");
    return 1;
}

/* `BO_DISPLAY=<n>` dans l'environnement, -1 sinon. */
static i64 capacite_affichage(u64 *pile)
{
    u64 argc = pile[0];
    char **envp = (char **)(pile + 1 + argc + 1);
    for (; *envp; envp++) {
        const char *e = *envp, *c = "BO_DISPLAY=";
        while (*c && *e == *c) { e++; c++; }
        if (*c) continue;
        i64 v = 0;
        if (!*e) return -1;
        while (*e >= '0' && *e <= '9') v = v * 10 + (*e++ - '0');
        return *e ? -1 : v;
    }
    return -1;
}

int principal(void)
{
    ecris("[COMPOSITED] tranchant vertical ring3 sur ABI native\n");

    /* --- 1. Le compositeur cree ses objets --------------------------------- */
    i64 region = sys6(BO_SHM_CREATE, OCTETS_REGION, 0, 0, 0, 0, 0);
    exige(region > 0, "region partagee creee");
    exige(sys6(BO_SHM_SIZE, (u64)region, 0, 0, 0, 0, 0) == (i64)OCTETS_REGION,
          "taille de la region conforme");

    u64 paire[2] = {0, 0};
    exige(sys6(BO_CHANNEL_CREATE, (u64)paire, 0, 0, 0, 0, 0) == 0, "canal cree");
    i64 cote_compositeur = (i64)paire[0];
    i64 cote_client = (i64)paire[1];
    exige(cote_compositeur > 0 && cote_client > 0, "deux extremites valides");

    i64 trame_prete = sys6(BO_EVENT_CREATE, 0, 0, 0, 0, 0, 0);
    exige(trame_prete > 0, "evenement de trame cree");

    i64 attente = sys6(BO_WAITSET_CREATE, 0, 0, 0, 0, 0, 0);
    exige(attente > 0, "waitset cree");
    exige(sys6(BO_WAITSET_ADD, (u64)attente, (u64)trame_prete, 7, 0, 0, 0) == 0,
          "evenement inscrit dans le waitset");

    /* --- 2. Le client demande une surface ---------------------------------- */
    unsigned char message[COMPOSITED_ENTETE + 64];
    co_pose_entete(message, COMPOSITED_DEMANDE_SURFACE, 8, 1);
    co_pose_u32(message + COMPOSITED_ENTETE, 0, LARGEUR);
    co_pose_u32(message + COMPOSITED_ENTETE, 4, HAUTEUR);
    exige(sys6(BO_CHANNEL_SEND, (u64)cote_client, (u64)message,
               COMPOSITED_ENTETE + 8, 0, 0, 0) >= 0,
          "demande de surface envoyee");

    unsigned char recu[COMPOSITED_ENTETE + 64];
    struct bo_recv_meta meta = {0, 0};
    i64 lu = sys6(BO_CHANNEL_RECV, (u64)cote_compositeur, (u64)recu, sizeof(recu),
                  0, 0, (u64)&meta);
    exige(lu > 0 && co_entete_valide(recu), "le compositeur recoit un message valide");
    exige(co_genre(recu) == COMPOSITED_DEMANDE_SURFACE, "et c'est bien la demande");

    /* --- 3. Le compositeur accorde, et ATTENUE les droits ------------------ */
    struct surface surface;
    surface.id = 1;
    surface.largeur = co_lit_u32(recu, COMPOSITED_ENTETE + 0);
    surface.hauteur = co_lit_u32(recu, COMPOSITED_ENTETE + 4);
    surface.pas = surface.largeur * 4;
    surface.proprietaires[0] = PROP_CLIENT;
    surface.proprietaires[1] = PROP_COMPOSITEUR;
    surface.affiche = -1;
    surface.degat_x = surface.degat_y = surface.degat_w = surface.degat_h = 0;
    surface.derniere_trame = 0;
    exige(surface.largeur == LARGEUR && surface.hauteur == HAUTEUR,
          "geometrie demandee transmise fidelement");

    /*
     * L'attenuation est le point du chantier 7 dont ce service depend : le
     * compositeur POSSEDE la region en lecture-ecriture, et n'en donne au
     * client qu'une capacite sans DUP ni TRANSFER. Le client peut dessiner ;
     * il ne peut ni multiplier la capacite ni la repasser plus loin.
     */
    u64 handles[1] = { (u64)region };
    u32 masques[1] = { BO_RIGHT_READ | BO_RIGHT_WRITE | BO_RIGHT_MAP | BO_RIGHT_INSPECT };
    unsigned char accordee[COMPOSITED_ENTETE + COMPOSITED_TAILLE_SURFACE_ACCORDEE];
    co_pose_entete(accordee, COMPOSITED_SURFACE_ACCORDEE,
                   COMPOSITED_TAILLE_SURFACE_ACCORDEE, 2);
    unsigned char *charge = accordee + COMPOSITED_ENTETE;
    co_pose_u32(charge, 0, surface.id);
    co_pose_u32(charge, 4, surface.largeur);
    co_pose_u32(charge, 8, surface.hauteur);
    co_pose_u32(charge, 12, surface.pas);
    co_pose_u32(charge, 16, COMPOSITED_ECHELLE_UNITE);
    co_pose_u32(charge, 20, COMPOSITED_TAMPONS);
    co_pose_u32(charge, 24, 0);
    co_pose_u32(charge, 28, 0); /* tampon initial */

    i64 envoye = sys6(BO_CHANNEL_SEND_ATTENUE, (u64)cote_compositeur, (u64)accordee,
                      sizeof(accordee), (u64)handles, 1, (u64)masques);
    exige(envoye >= 0, "surface accordee, region transferee attenuee");

    struct bo_recv_meta meta_client = {0, 0};
    u64 recus[1] = {0};
    unsigned char rep[COMPOSITED_ENTETE + COMPOSITED_TAILLE_SURFACE_ACCORDEE];
    lu = sys6(BO_CHANNEL_RECV, (u64)cote_client, (u64)rep, sizeof(rep),
              (u64)recus, 1, (u64)&meta_client);
    exige(lu > 0 && co_genre(rep) == COMPOSITED_SURFACE_ACCORDEE,
          "le client recoit sa surface");
    exige(meta_client.handles == 1, "et la capacite qui va avec");

    struct bo_handle_info info = {0, 0, 0, 0};
    exige(sys6(BO_HANDLE_INFO, recus[0], (u64)&info, 0, 0, 0, 0) == 0,
          "la capacite recue est inspectable");
    exige((info.rights & BO_RIGHT_WRITE) != 0, "le client peut dessiner");
    exige((info.rights & BO_RIGHT_DUP) == 0,
          "le client NE PEUT PAS multiplier la capacite");
    exige((info.rights & BO_RIGHT_TRANSFER) == 0,
          "ni la repasser a un tiers");
    exige(sys6(BO_HANDLE_DUP, recus[0], BO_RIGHT_READ, 0, 0, 0, 0) < 0,
          "et la duplication est effectivement refusee");

    /* --- 4. Le client dessine dans le tampon qu'il possede ------------------ */
    u32 tampon_client = co_lit_u32(rep, COMPOSITED_ENTETE + 28);
    exige(tampon_client == 0, "le client possede le tampon initial");
    u32 decalage = tampon_client * OCTETS_TAMPON;

    unsigned char ligne[PAS];
    for (unsigned i = 0; i < PAS; i += 4) {
        ligne[i + 0] = 0x20; ligne[i + 1] = 0x40;
        ligne[i + 2] = 0x80; ligne[i + 3] = 0xFF;
    }
    i64 ecrit = sys6(BO_SHM_WRITE, recus[0], decalage, (u64)ligne, PAS, 0, 0);
    exige(ecrit == (i64)PAS, "le client ecrit une ligne dans son tampon");

    /* --- 5. Trame livree, evenement signale -------------------------------- */
    unsigned char livree[COMPOSITED_ENTETE + COMPOSITED_TAILLE_TRAME_LIVREE];
    co_pose_entete(livree, COMPOSITED_TRAME_LIVREE,
                   COMPOSITED_TAILLE_TRAME_LIVREE, 3);
    charge = livree + COMPOSITED_ENTETE;
    co_pose_u32(charge, 0, surface.id);
    co_pose_u32(charge, 4, tampon_client);
    co_pose_u32(charge, 8, 1);          /* numero de trame */
    co_pose_u32(charge, 12, 0);         /* degat.x */
    co_pose_u32(charge, 16, 0);         /* degat.y */
    co_pose_u32(charge, 20, LARGEUR);   /* degat.largeur */
    co_pose_u32(charge, 24, 1);         /* degat.hauteur */
    exige(sys6(BO_CHANNEL_SEND, (u64)cote_client, (u64)livree, sizeof(livree),
               0, 0, 0) >= 0, "trame livree envoyee");
    exige(sys6(BO_EVENT_SIGNAL, (u64)trame_prete, 0, 0, 0, 0, 0) == 0,
          "evenement de trame signale");

    /* --- 6. Le compositeur attend, compose, presente ------------------------ */
    struct bo_wait_event evenements[4];
    i64 prets = sys6(BO_WAITSET_POLL, (u64)attente, (u64)evenements, 4, 0, 0, 0);
    exige(prets == 1, "le waitset reveille le compositeur");
    exige(evenements[0].key == 7, "et nomme la source du reveil");
    exige((evenements[0].signals & BO_SIGNAL_SIGNALED) != 0, "l'evenement est arme");

    unsigned char trame[COMPOSITED_ENTETE + COMPOSITED_TAILLE_TRAME_LIVREE];
    lu = sys6(BO_CHANNEL_RECV, (u64)cote_compositeur, (u64)trame, sizeof(trame),
              0, 0, (u64)&meta);
    exige(lu > 0 && co_genre(trame) == COMPOSITED_TRAME_LIVREE,
          "le compositeur recoit la trame");

    charge = trame + COMPOSITED_ENTETE;
    u32 tampon = co_lit_u32(charge, 4);
    exige(tampon < COMPOSITED_TAMPONS
          && surface.proprietaires[tampon] == PROP_CLIENT,
          "le tampon livre appartenait bien au client");
    surface.proprietaires[tampon] = PROP_COMPOSITEUR;
    surface.derniere_trame = co_lit_u32(charge, 8);
    accumule_degat(&surface, co_lit_u32(charge, 12), co_lit_u32(charge, 16),
                   co_lit_u32(charge, 20), co_lit_u32(charge, 24));
    exige(surface.degat_w == LARGEUR && surface.degat_h == 1,
          "le degat est accumule, rogne a la surface");

    /* Le compositeur LIT les pixels du tampon livre : c'est la composition. */
    unsigned char relu[PAS];
    i64 pris = sys6(BO_SHM_READ, (u64)region, tampon * OCTETS_TAMPON,
                    (u64)relu, PAS, 0, 0);
    exige(pris == (i64)PAS, "le compositeur relit le tampon livre");
    int identique = 1;
    for (unsigned i = 0; i < PAS; i++) if (relu[i] != ligne[i]) identique = 0;
    exige(identique,
          "les pixels ecrits par le client sont ceux que le compositeur compose");
    pixels_composes += surface.degat_w * surface.degat_h;
    trames_presentees++;

    /* --- 7. Le tampon libere revient au client ----------------------------- */
    int ancien = surface.affiche;
    surface.proprietaires[tampon] = PROP_AFFICHE;
    surface.affiche = (int)tampon;
    u32 rendu = 0;
    int a_rendu = 0;
    if (ancien >= 0 && (u32)ancien != tampon) {
        surface.proprietaires[ancien] = PROP_CLIENT;
        rendu = (u32)ancien; a_rendu = 1;
    } else {
        for (u32 autre = 0; autre < COMPOSITED_TAMPONS; autre++) {
            if (autre != tampon && surface.proprietaires[autre] == PROP_COMPOSITEUR) {
                surface.proprietaires[autre] = PROP_CLIENT;
                rendu = autre; a_rendu = 1;
            }
        }
    }
    exige(a_rendu, "un tampon est rendu au client apres la presentation");

    unsigned char annonce[COMPOSITED_ENTETE + COMPOSITED_TAILLE_TAMPON_RENDU];
    co_pose_entete(annonce, COMPOSITED_TAMPON_RENDU,
                   COMPOSITED_TAILLE_TAMPON_RENDU, 4);
    charge = annonce + COMPOSITED_ENTETE;
    co_pose_u32(charge, 0, surface.id);
    co_pose_u32(charge, 4, rendu);
    co_pose_u32(charge, 8, surface.derniere_trame);
    co_pose_u32(charge, 12, 0);
    exige(sys6(BO_CHANNEL_SEND, (u64)cote_compositeur, (u64)annonce,
               sizeof(annonce), 0, 0, 0) >= 0, "annonce du tampon rendu envoyee");

    unsigned char recu_rendu[COMPOSITED_ENTETE + COMPOSITED_TAILLE_TAMPON_RENDU];
    lu = sys6(BO_CHANNEL_RECV, (u64)cote_client, (u64)recu_rendu,
              sizeof(recu_rendu), 0, 0, (u64)&meta_client);
    exige(lu > 0 && co_genre(recu_rendu) == COMPOSITED_TAMPON_RENDU,
          "le client apprend qu'il peut redessiner");
    exige(co_lit_u32(recu_rendu, COMPOSITED_ENTETE + 4) == rendu,
          "et sur quel tampon");

    /* --- Ce qu'un compositeur doit REFUSER ---------------------------------- */
    exige(surface.proprietaires[tampon] == PROP_AFFICHE,
          "le tampon presente n'appartient plus au client");

    sys6(BO_EVENT_RESET, (u64)trame_prete, 0, 0, 0, 0, 0);
    prets = sys6(BO_WAITSET_POLL, (u64)attente, (u64)evenements, 4, 0, 0, 0);
    exige(prets == 0, "sans nouvelle trame, le compositeur ne se reveille pas");

    /* --- Verdict ------------------------------------------------------------ */
    ecris_nombre("[COMPOSITED] pixels_composes=", pixels_composes);
    ecris_nombre("[COMPOSITED] trames_presentees=", trames_presentees);
    ecris_nombre("[COMPOSITED] echecs=", echecs);

    sys6(BO_HANDLE_CLOSE, (u64)cote_client, 0, 0, 0, 0, 0);
    sys6(BO_HANDLE_CLOSE, (u64)cote_compositeur, 0, 0, 0, 0, 0);
    sys6(BO_HANDLE_CLOSE, (u64)region, 0, 0, 0, 0, 0);

    if (echecs == 0) {
        ecris("COMPOSITED_SLICE_OK\n");
        return 0;
    }
    ecris("COMPOSITED_SLICE_FAIL\n");
    return 1;
}

/* L'ENTREE D'UN PROCESSUS N'EST PAS UN APPEL DE FONCTION.
 *
 * Le noyau SAUTE ici avec une pile alignee sur seize octets. Une fonction C,
 * elle, est compilee en supposant qu'un `call` vient d'empiler une adresse de
 * retour -- donc que `rsp` vaut huit modulo seize a son entree. Ecrire `_start`
 * en C ordinaire decale ainsi TOUT le reste de huit octets.
 *
 * Le decalage ne se voit nulle part jusqu'a la premiere instruction qui exige
 * vraiment l'alignement. Ici, c'etait un `movaps %xmm0,0x60(%rsp)` emis par le
 * compilateur pour initialiser une structure locale :
 *
 *     faute de protection generale en ring 3 : rip=0x4000004012b7
 *                                              rsp=0x5ffbfffff5b8
 *
 * 0x5b8 + 0x60 = 0x618, soit huit modulo seize. `movaps` sur une adresse non
 * alignee leve un #GP, et le processus mourait apres trois etapes reussies.
 *
 * `native-ipc-ring3-probe` avait deja ce motif, et pour cette raison. On le
 * reprend a l'identique : realigner explicitement, puis appeler.
 */
void _start(void)
{
    __asm__ volatile(
        "movq %rsp, %rdi\n\t"
        "andq $-16, %rsp\n\t"
        "call demarre\n\t"
        "ud2\n\t"
    );
}

/* Appelee par `_start` avec une pile conforme a l'ABI. */
void demarre(u64 *pile);
void demarre(u64 *pile)
{
    i64 affichage = capacite_affichage(pile);
    int code;
    if (affichage >= 0) {
        /* `/bin/composited presentation`, lance par le noyau. */
        code = presentation(affichage);
    } else {
        code = principal();
        if (presentation(-1) != 0) code = 1;
    }
    sys6(231, (u64)code, 0, 0, 0, 0, 0); /* exit_group */
    __builtin_unreachable();
}
