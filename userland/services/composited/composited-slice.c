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
#define BO_DISPLAY_INFO    (BO_BASE + 0x81)
#define BO_FENETRE_POMPE   (BO_BASE + 0x90)
#define BO_FENETRE_LIT     (BO_BASE + 0x91)
#define BO_FENETRE_CONFIGURE (BO_BASE + 0x92)
#define BO_FENETRE_FERME   (BO_BASE + 0x93)
#define BO_DISPLAY_RELANCE_CLIENT (BO_BASE + 0x82)
#define BO_WAITSET_REMOVE  (BO_BASE + 0x32)
#define BO_WAITSET_WAIT    (BO_BASE + 0x34)
#define BO_TIME_NOW        (BO_BASE + 0x60)

#define BO_RIGHT_READ     (1u << 0)
#define BO_RIGHT_WRITE    (1u << 1)
#define BO_RIGHT_MAP      (1u << 3)
#define BO_RIGHT_DUP      (1u << 4)
#define BO_RIGHT_TRANSFER (1u << 5)
#define BO_RIGHT_INSPECT  (1u << 6)
#define BO_E_INVALID_ARGUMENT 2
#define BO_E_ACCESS_DENIED    5
#define BO_E_FAULT           11
#define BO_E_TIMED_OUT       14
#define BO_SIGNAL_READABLE    (1u << 0)
#define BO_SIGNAL_PEER_CLOSED (1u << 3)

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

/* `NOM=<n>` (decimal) dans l'environnement, -1 sinon. */
static i64 variable_nombre(u64 *pile, const char *nom)
{
    u64 argc = pile[0];
    char **envp = (char **)(pile + 1 + argc + 1);
    for (; *envp; envp++) {
        const char *e = *envp, *c = nom;
        while (*c && *e == *c) { e++; c++; }
        if (*c) continue;
        i64 v = 0;
        if (!*e) return -1;
        while (*e >= '0' && *e <= '9') v = v * 10 + (*e++ - '0');
        return *e ? -1 : v;
    }
    return -1;
}

/* `BO_DISPLAY=<n>` dans l'environnement, -1 sinon. */
static i64 capacite_affichage(u64 *pile)
{
    return variable_nombre(pile, "BO_DISPLAY=");
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

/* --- BOUCHAUD_BUREAU_RING3_2A/2B : de vraies fenetres -----------------------
 *
 * Lance par le noyau en mode `bureau` quand l'image configure un bureau ring 3
 * (`/bureau-ring3`). Le service recoit :
 *   - `BO_DISPLAY`  : poser des rectangles a l'ecran ;
 *   - `BO_FENETRES` : une capacite `Fenetre` par application BOGU ordinaire,
 *                     lancee par le noyau dans SON processus (du fond vers le
 *                     premier plan) ;
 *   - `BO_BUREAU_SCENARIO` (facultatif) : `deplace-empile` deplace la fenetre
 *     du fond puis la remonte au premier plan (pas d'entrees avant la 2C).
 * Il compose le bureau dans SA region (l'image de l'ecran) par l'algorithme du
 * peintre : sur un rectangle abime, le fond, puis chaque fenetre du fond vers
 * le premier plan -- cadre, puis contenu recopie par le noyau
 * (`bo_fenetre_lit`, bornes jugees). Il ne presente que les rectangles abimes.
 * Il ne voit ni les surfaces, ni les processus des applications.
 *
 * Marqueurs : COMPOSITED_BUREAU_PRET, COMPOSITED_BUREAU_FENETRE,
 * COMPOSITED_BUREAU_PREMIERE_TRAME, COMPOSITED_BUREAU_MESURE,
 * COMPOSITED_BUREAU_REFUS_OK, COMPOSITED_BUREAU_SCENARIO(_FIN),
 * COMPOSITED_BUREAU_CLIENT_MORT, COMPOSITED_BUREAU_BILAN, COMPOSITED_BUREAU_OK.
 */

#define BUREAU_FOND     0x1e3a5fu
#define BUREAU_BORD     0x0b1a2au
#define BUREAU_BARRE    0x2f6fb0u
#define BUREAU_BARRE_INACTIVE 0x46607au
#define BUREAU_BARRE_H  24u
#define BUREAU_BORD_L   2u
#define BUREAU_LIGNE_MAX 4096u
#define FEN_MAX 4

static u32 ecran_l, ecran_h;
static u32 ligne_bureau[BUREAU_LIGNE_MAX];

struct fen {
    i64 poignee;
    u32 x, y;             /* coin du CONTENU, en pixels ecran */
    u32 l, h;             /* taille du contenu (ce que le client a peint) */
    u32 pid;
    int vivante, visible, minimisee;
    u64 trames;           /* trames NOUVELLES composees (revue #298) */
    u32 livrees;          /* dernier compte de trames livrees lu par la pompe */
};
static struct fen fens[FEN_MAX];
static int nfen;
static int ordre[FEN_MAX];   /* ordre[0] : fond ... ordre[nfen-1] : premier plan */
static i64 affichage_g, region_g;
static u64 lit_ns, lit_max, pres_ns, pres_max, trames_total, pixels_total, recompositions;

struct rect { u32 x, y, l, h; };

static struct rect cadre_de(const struct fen *f)
{
    struct rect r = { f->x - BUREAU_BORD_L, f->y - BUREAU_BARRE_H - BUREAU_BORD_L,
                      f->l + 2 * BUREAU_BORD_L, f->h + BUREAU_BARRE_H + 2 * BUREAU_BORD_L };
    return r;
}

static int intersecte(struct rect a, struct rect b, struct rect *out)
{
    u32 x0 = a.x > b.x ? a.x : b.x, y0 = a.y > b.y ? a.y : b.y;
    u32 x1 = (a.x + a.l) < (b.x + b.l) ? (a.x + a.l) : (b.x + b.l);
    u32 y1 = (a.y + a.h) < (b.y + b.h) ? (a.y + a.h) : (b.y + b.h);
    if (x1 <= x0 || y1 <= y0) return 0;
    out->x = x0; out->y = y0; out->l = x1 - x0; out->h = y1 - y0;
    return 1;
}

static i64 ecris_segment(u32 x, u32 y, u32 l, u32 couleur)
{
    for (u32 c = 0; c < l; c++) ligne_bureau[c] = couleur;
    return sys6(BO_SHM_WRITE, (u64)region_g, ((u64)y * ecran_l + x) * 4u, (u64)ligne_bureau, (u64)l * 4u, 0, 0);
}

static i64 remplit(struct rect r, u32 couleur)
{
    for (u32 y = r.y; y < r.y + r.h; y++) {
        i64 e = ecris_segment(r.x, y, r.l, couleur);
        if (e < 0) return e;
    }
    return 0;
}

struct lecture_fenetre { u32 x, y, l, h, decalage, pas; };

static i64 lit_fenetre(i64 fenetre, i64 region, u32 x, u32 y, u32 l, u32 h, u32 dx, u32 dy)
{
    struct lecture_fenetre d = { x, y, l, h, (dy * ecran_l + dx) * 4u, ecran_l * 4u };
    return sys6(BO_FENETRE_LIT, (u64)fenetre, (u64)region, (u64)&d, 0, 0, 0);
}

static u64 maintenant_ns(void) { return (u64)sys6(BO_TIME_NOW, 0, 0, 0, 0, 0, 0); }

static i64 presente_rect(struct rect r)
{
    struct demande_presentation d = { ecran_l, ecran_h, r.x, r.y, r.l, r.h, r.x, r.y };
    u64 t0 = maintenant_ns();
    i64 p = sys6(BO_DISPLAY_PRESENT, (u64)affichage_g, (u64)region_g, (u64)&d, 0, 0, 0);
    u64 dt = maintenant_ns() - t0;
    pres_ns += dt; if (dt > pres_max) pres_max = dt;
    return p;
}

/* Contenu de la fenetre `f` sur le rectangle ecran `r` (deja dans le contenu). */
static i64 recopie_contenu(struct fen *f, struct rect r)
{
    u64 t0 = maintenant_ns();
    i64 n = lit_fenetre(f->poignee, region_g, r.x - f->x, r.y - f->y, r.l, r.h, r.x, r.y);
    u64 dt = maintenant_ns() - t0;
    lit_ns += dt; if (dt > lit_max) lit_max = dt;
    if (n > 0) pixels_total += (u64)n;
    return n;
}

/* La fenetre ACTIVE : la plus haute fenetre VISIBLE (une fenetre minimisee
 * ou morte au sommet de la pile ne garde pas la barre active), -1 si aucune. */
static int active(void)
{
    for (int k = nfen - 1; k >= 0; k--)
        if (fens[ordre[k]].visible) return ordre[k];
    return -1;
}

/* L'algorithme du peintre sur `r`, puis UNE presentation. */
static i64 recompose(struct rect r)
{
    struct rect ecran = { 0, 0, ecran_l, ecran_h };
    if (!intersecte(r, ecran, &r)) return 0;
    recompositions++;
    i64 e = remplit(r, BUREAU_FOND);
    if (e < 0) return e;
    for (int k = 0; k < nfen; k++) {
        struct fen *f = &fens[ordre[k]];
        if (!f->visible) continue;
        struct rect c = cadre_de(f), i;
        if (!intersecte(r, c, &i)) continue;
        int premier_plan = (ordre[k] == active());
        /* Cadre : bordure, barre (active au premier plan), en bandes. */
        struct rect barre = { f->x, f->y - BUREAU_BARRE_H, f->l, BUREAU_BARRE_H };
        struct rect contenu = { f->x, f->y, f->l, f->h };
        struct rect morceau;
        if (intersecte(i, c, &morceau)) {
            for (u32 y = morceau.y; y < morceau.y + morceau.h; y++) {
                struct rect ligne = { morceau.x, y, morceau.l, 1 }, sous;
                if (intersecte(ligne, barre, &sous)) {
                    /* bordure gauche/droite autour de la barre */
                    if (sous.x > ligne.x) ecris_segment(ligne.x, y, sous.x - ligne.x, BUREAU_BORD);
                    ecris_segment(sous.x, y, sous.l, premier_plan ? BUREAU_BARRE : BUREAU_BARRE_INACTIVE);
                    if (sous.x + sous.l < ligne.x + ligne.l)
                        ecris_segment(sous.x + sous.l, y, ligne.x + ligne.l - sous.x - sous.l, BUREAU_BORD);
                } else if (intersecte(ligne, contenu, &sous)) {
                    if (sous.x > ligne.x) ecris_segment(ligne.x, y, sous.x - ligne.x, BUREAU_BORD);
                    if (sous.x + sous.l < ligne.x + ligne.l)
                        ecris_segment(sous.x + sous.l, y, ligne.x + ligne.l - sous.x - sous.l, BUREAU_BORD);
                } else {
                    ecris_segment(ligne.x, y, ligne.l, BUREAU_BORD);
                }
            }
        }
        if (intersecte(i, contenu, &morceau)) recopie_contenu(f, morceau);
    }
    return presente_rect(r);
}

static int rang_de(int indice)
{
    for (int k = 0; k < nfen; k++) if (ordre[k] == indice) return k;
    return -1;
}

/* Une fenetre au-dessus de `indice` recouvre-t-elle `r` ? */
static int recouverte(int indice, struct rect r)
{
    struct rect i;
    for (int k = rang_de(indice) + 1; k < nfen; k++) {
        struct fen *f = &fens[ordre[k]];
        if (f->visible && intersecte(r, cadre_de(f), &i)) return 1;
    }
    return 0;
}

static void remonte(int indice)
{
    int k = rang_de(indice);
    if (k < 0) return;
    int ancien_haut = active();   /* la fenetre active AVANT, pas le sommet de pile */
    for (; k < nfen - 1; k++) ordre[k] = ordre[k + 1];
    ordre[nfen - 1] = indice;
    recompose(cadre_de(&fens[indice]));
    if (ancien_haut >= 0 && ancien_haut != indice && fens[ancien_haut].visible) {
        struct rect b = { fens[ancien_haut].x, fens[ancien_haut].y - BUREAU_BARRE_H, fens[ancien_haut].l, BUREAU_BARRE_H };
        recompose(b);   /* sa barre devient inactive */
    }
}

static void deplace(int indice, u32 nx, u32 ny)
{
    struct fen *f = &fens[indice];
    struct rect avant = cadre_de(f);
    f->x = nx; f->y = ny;
    recompose(avant);
    recompose(cadre_de(f));
}

/* Les refus que le noyau doit opposer, une fois une trame connue. */
static void eprouve_refus(i64 fenetre, u32 peint_l, u32 peint_h, u32 fx, u32 fy)
{
    i64 region = region_g, affichage = affichage_g;
    i64 r = lit_fenetre(fenetre, region, 1, 0, peint_l, 1, fx, fy);
    exige(r == -BO_E_INVALID_ARGUMENT, "bureau : lecture au-dela de ce que le client a peint refusee");
    r = lit_fenetre(fenetre, region, 0, peint_h, 1, 1, fx, fy);
    exige(r == -BO_E_INVALID_ARGUMENT, "bureau : ligne sous la surface peinte refusee");
    struct lecture_fenetre geant = { 0, 0, 1, 1, 0xfffffff0u, 4 };
    r = sys6(BO_FENETRE_LIT, (u64)fenetre, (u64)region, (u64)&geant, 0, 0, 0);
    exige(r == -BO_E_INVALID_ARGUMENT, "bureau : destination hors region (debordement u32) refusee");
    struct lecture_fenetre pas_court = { 0, 0, 8, 2, 0, 4 };
    r = sys6(BO_FENETRE_LIT, (u64)fenetre, (u64)region, (u64)&pas_court, 0, 0, 0);
    exige(r == -BO_E_INVALID_ARGUMENT, "bureau : pas plus court qu'une ligne refuse");
    i64 lecture_seule = sys6(BO_HANDLE_DUP, (u64)region, BO_RIGHT_READ, 0, 0, 0, 0);
    exige(lecture_seule > 0, "bureau : region dupliquee en lecture seule");
    r = lit_fenetre(fenetre, lecture_seule, 0, 0, 1, 1, fx, fy);
    exige(r == -BO_E_ACCESS_DENIED, "bureau : ecriture dans une region sans droit WRITE refusee");
    sys6(BO_HANDLE_CLOSE, (u64)lecture_seule, 0, 0, 0, 0, 0);
    r = lit_fenetre(affichage, region, 0, 0, 1, 1, fx, fy);
    exige(r == -BO_E_ACCESS_DENIED, "bureau : la capacite d'affichage ne sert pas de fenetre");
    r = sys6(BO_HANDLE_DUP, (u64)fenetre, BO_RIGHT_READ, 0, 0, 0, 0);
    exige(r < 0, "bureau : la capacite fenetre ne se duplique pas");
    r = sys6(BO_FENETRE_POMPE, (u64)fenetre, 0xffff800000000000UL, 0, 0, 0, 0);
    exige(r == -BO_E_FAULT, "bureau : etat vers une adresse noyau refuse");
    struct lecture_fenetre *invalide = (struct lecture_fenetre *)0x10;
    r = sys6(BO_FENETRE_LIT, (u64)fenetre, (u64)region, (u64)invalide, 0, 0, 0);
    exige(r == -BO_E_FAULT, "bureau : demande a une adresse invalide refusee");
    if (echecs == 0) ecris("COMPOSITED_BUREAU_REFUS_OK\n");
}

/* Place la fenetre `i` (taille connue) : en cascade, dans l'ecran. */
static void place(int i)
{
    struct fen *f = &fens[i];
    u32 x = 40 + (u32)i * 420, y = 40 + BUREAU_BARRE_H + BUREAU_BORD_L + (u32)i * 190;
    if (x + f->l + BUREAU_BORD_L > ecran_l) x = ecran_l > f->l + BUREAU_BORD_L ? ecran_l - f->l - BUREAU_BORD_L : BUREAU_BORD_L;
    if (y + f->h + BUREAU_BORD_L > ecran_h) y = ecran_h > f->h + BUREAU_BORD_L ? ecran_h - f->h - BUREAU_BORD_L : BUREAU_BARRE_H + BUREAU_BORD_L;
    if (x < BUREAU_BORD_L) x = BUREAU_BORD_L;
    if (y < BUREAU_BARRE_H + BUREAU_BORD_L) y = BUREAU_BARRE_H + BUREAU_BORD_L;
    f->x = x; f->y = y;
}

/* `COMPOSITED_BUREAU_TRAMES fenetre=I pid=P trames=N`, en UNE ecriture. */
static void ajoute_nombre(char *t, unsigned long *n, u64 v)
{
    char c[24]; int k = 0;
    if (v == 0) c[k++] = '0';
    while (v) { c[k++] = (char)('0' + v % 10); v /= 10; }
    while (k && *n < 180) t[(*n)++] = c[--k];
}

static void ajoute_texte(char *t, unsigned long *n, const char *s)
{
    while (*s && *n < 180) t[(*n)++] = *s++;
}

static void publie_trames(int i)
{
    char t[192]; unsigned long n = 0;
    ajoute_texte(t, &n, "COMPOSITED_BUREAU_TRAMES fenetre=");
    ajoute_nombre(t, &n, (u64)i);
    ajoute_texte(t, &n, " pid=");
    ajoute_nombre(t, &n, fens[i].pid);
    ajoute_texte(t, &n, " trames=");
    ajoute_nombre(t, &n, fens[i].trames);
    ajoute_texte(t, &n, fens[i].vivante ? " vivante=1\n" : " vivante=0\n");
    t[n] = 0;
    ecris(t);
}

static void publie_fenetre(int i, const char *etiquette)
{
    struct fen *f = &fens[i];
    ecris_nombre(etiquette, i);
    ecris_nombre("[COMPOSITED] pid=", f->pid);
    ecris_nombre("[COMPOSITED] x=", f->x);
    ecris_nombre("[COMPOSITED] y=", f->y);
    ecris_nombre("[COMPOSITED] l=", f->l);
    ecris_nombre("[COMPOSITED] h=", f->h);
    ecris_nombre("[COMPOSITED] rang=", rang_de(i));
}

/* `BO_FENETRES=h1,h2,...` dans l'environnement. */
static int lit_fenetres(u64 *pile)
{
    u64 argc = pile[0];
    char **envp = (char **)(pile + 1 + argc + 1);
    for (; *envp; envp++) {
        const char *e = *envp, *c = "BO_FENETRES=";
        while (*c && *e == *c) { e++; c++; }
        if (*c) continue;
        while (*e && nfen < FEN_MAX) {
            i64 v = 0; int chiffres = 0;
            while (*e >= '0' && *e <= '9') { v = v * 10 + (*e++ - '0'); chiffres++; }
            if (!chiffres) return 0;
            fens[nfen].poignee = v; ordre[nfen] = nfen; nfen++;
            if (*e == ',') e++; else if (*e) return 0;
        }
        return nfen;
    }
    return 0;
}

/* 0 : aucun ; 1 : `deplace-empile` ; 2 : `complet` (deplace-empile, puis
 * redimensionnement, minimisation, restauration, fermeture, relance). */
static int scenario_de(u64 *pile)
{
    u64 argc = pile[0];
    char **envp = (char **)(pile + 1 + argc + 1);
    for (; *envp; envp++) {
        const char *noms[2] = { "BO_BUREAU_SCENARIO=deplace-empile", "BO_BUREAU_SCENARIO=complet" };
        for (int k = 0; k < 2; k++) {
            const char *e = *envp, *c = noms[k];
            while (*c && *e == *c) { e++; c++; }
            if (!*c && !*e) return k + 1;
        }
    }
    return 0;
}

/* La barre de titre de la fenetre `i` (pour un changement de fenetre active). */
static void recompose_barre(int i)
{
    if (i < 0 || !fens[i].visible) return;
    struct rect b = { fens[i].x, fens[i].y - BUREAU_BARRE_H, fens[i].l, BUREAU_BARRE_H };
    recompose(b);
}

static void minimise(int i)
{
    struct fen *f = &fens[i];
    if (!f->visible) return;
    f->visible = 0; f->minimisee = 1;
    recompose(cadre_de(f));
    recompose_barre(active());   /* la fenetre qui devient active */
    ecris_nombre("COMPOSITED_BUREAU_MINIMISE fenetre=", i);
}

static void restaure(int i)
{
    struct fen *f = &fens[i];
    if (!f->minimisee) return;
    int avant = active();
    f->minimisee = 0; f->visible = 1;
    recompose(cadre_de(f));
    if (avant != active()) recompose_barre(avant);   /* elle cesse d'etre active */
    publie_fenetre(i, "COMPOSITED_BUREAU_RESTAURE fenetre=");
}

static int bureau(i64 affichage, u64 *pile)
{
    affichage_g = affichage;
    if (lit_fenetres(pile) <= 0) { ecris("COMPOSITED_BUREAU_FAIL raison=fenetres\n"); return 1; }
    int scenario = scenario_de(pile);
    u64 attente_etape = 0;   /* instant (ns) de l'etape suivante du scenario complet */
    i64 dims = sys6(BO_DISPLAY_INFO, (u64)affichage, 0, 0, 0, 0, 0);
    exige(dims > 0, "bureau : taille de l'ecran connue");
    if (dims <= 0) { ecris("COMPOSITED_BUREAU_FAIL raison=ecran\n"); return 1; }
    ecran_l = (u32)((u64)dims >> 32); ecran_h = (u32)((u64)dims & 0xffffffffu);
    if (ecran_l > BUREAU_LIGNE_MAX || ecran_l < 800 || ecran_h < 600) {
        ecris_nombre("COMPOSITED_BUREAU_FAIL raison=ecran-hors-gabarit l=", ecran_l);
        return 1;
    }
    region_g = sys6(BO_SHM_CREATE, (u64)ecran_l * ecran_h * 4u, 0, 0, 0, 0, 0);
    exige(region_g > 0, "bureau : image de l'ecran creee");
    if (region_g <= 0) return 1;
    struct rect tout = { 0, 0, ecran_l, ecran_h };
    exige(recompose(tout) >= 0, "bureau : fond presente");

    i64 attente = sys6(BO_WAITSET_CREATE, 0, 0, 0, 0, 0, 0);
    int ajoutees = 0;
    for (int i = 0; i < nfen; i++) {
        fens[i].vivante = 1;
        if (attente > 0 && sys6(BO_WAITSET_ADD, (u64)attente, (u64)fens[i].poignee, (u64)i + 1, 0, 0, 0) == 0) ajoutees++;
    }
    exige(ajoutees == nfen, "bureau : chaque fenetre attendue dans un waitset");
    ecris_nombre("COMPOSITED_BUREAU_PRET fenetres=", nfen);
    ecris_nombre("[COMPOSITED] ecran_l=", ecran_l);
    ecris_nombre("[COMPOSITED] ecran_h=", ecran_h);

    struct bo_wait_event evenements[FEN_MAX];
    u64 premiere = 0, debut_ns = 0;
    int refus_eprouves = 0, etape = 0, vivantes = nfen;
    u32 etat[10];
    while (vivantes > 0) {
        i64 n = sys6(BO_WAITSET_WAIT, (u64)attente, (u64)evenements, FEN_MAX, 1000000000UL, 0, 0);
        if (n < 0 && n != -BO_E_TIMED_OUT) { ecris_nombre("COMPOSITED_BUREAU_FAIL attente=", n); return 1; }
        for (int i = 0; i < nfen; i++) {
            struct fen *f = &fens[i];
            if (!f->vivante) continue;
            i64 v = sys6(BO_FENETRE_POMPE, (u64)f->poignee, (u64)etat, 0, 0, 0, 0);
            if (v < 0) { ecris_nombre("COMPOSITED_BUREAU_FAIL pompe=", v); return 1; }
            f->pid = etat[1];
            /* Revue #298 : une trame n'est comptee que si le client en a
             * LIVRE une nouvelle (compte du noyau, etat[8]). Un reveil du
             * waitset cause par un autre client ne fait plus avancer le
             * compte d'une fenetre figee. */
            u32 nouvelles = etat[8] - f->livrees;
            f->livrees = etat[8];
            if (v == 0) {
                /* Mort : la fenetre disparait, les autres continuent. */
                f->vivante = 0; vivantes--;
                ecris_nombre("COMPOSITED_BUREAU_CLIENT_MORT pid=", f->pid);
                sys6(BO_WAITSET_REMOVE, (u64)attente, (u64)i + 1, 0, 0, 0, 0);
                sys6(BO_HANDLE_CLOSE, (u64)f->poignee, 0, 0, 0, 0, 0);
                if (f->visible) {
                    f->visible = 0;
                    exige(recompose(cadre_de(f)) >= 0, "bureau : fenetre du client mort effacee");
                    recompose_barre(active());   /* la fenetre qui devient active */
                }
                continue;
            }
            u32 dx = etat[4], dy = etat[5], dl = etat[6], dh = etat[7];
            if (!f->visible) {
                /* Minimisee : ses trames sont pompees (le client avance), pas dessinees. */
                if (f->minimisee) { if (nouvelles) { f->trames++; trames_total++; } continue; }
                if (etat[2] == 0 || etat[3] == 0) continue;
                f->l = etat[2]; f->h = etat[3];
                place(i);
                int avant = active();
                f->visible = 1;
                publie_fenetre(i, "COMPOSITED_BUREAU_FENETRE indice=");
                recompose(cadre_de(f));
                if (avant >= 0 && avant != active()) recompose_barre(avant);   /* elle cesse d'etre active */
                dl = 0;
            } else if (etat[2] != f->l || etat[3] != f->h) {
                /* Taille peinte changee : tout le cadre, ancien et nouveau. */
                struct rect avant = cadre_de(f);
                f->l = etat[2]; f->h = etat[3];
                recompose(avant); recompose(cadre_de(f));
                publie_fenetre(i, "COMPOSITED_BUREAU_FENETRE indice=");
                dl = 0;
            }
            if (dl && dh) {
                if (dx + dl > f->l) dl = dx < f->l ? f->l - dx : 0;
                if (dy + dh > f->h) dh = dy < f->h ? f->h - dy : 0;
            }
            /* Revue #298 : le dommage est coupe a l'ecran, comme dans
             * `recompose` : une fenetre plus large que l'ecran ne fait plus
             * demander au noyau une lecture ou une presentation hors bornes
             * (refusees, contenu visible perime). */
            struct rect d = { f->x + dx, f->y + dy, dl, dh }, ecran = { 0, 0, ecran_l, ecran_h };
            if (dl && dh && !intersecte(d, ecran, &d)) dl = 0;
            if (dl && dh) {
                if (recouverte(i, d)) {
                    recompose(d);
                } else {
                    if (recopie_contenu(f, d) < 0) continue;
                    if (presente_rect(d) < 0) continue;
                }
            }
            if (!nouvelles) continue;
            f->trames++; trames_total++;
            if (!premiere) {
                premiere = 1; debut_ns = maintenant_ns();
                ecris_nombre("COMPOSITED_BUREAU_PREMIERE_TRAME fenetre=", i);
            }
            if (!refus_eprouves && f->trames >= 3) {
                refus_eprouves = 1;
                eprouve_refus(f->poignee, f->l, f->h, f->x, f->y);
            }
            if (trames_total % 120 == 0) {
                ecris_nombre("COMPOSITED_BUREAU_MESURE trames=", (i64)trames_total);
                ecris_nombre("[COMPOSITED] pixels_copies=", (i64)pixels_total);
                ecris_nombre("[COMPOSITED] lit_us_moy=", (i64)(lit_ns / trames_total / 1000));
                ecris_nombre("[COMPOSITED] lit_us_max=", (i64)(lit_max / 1000));
                ecris_nombre("[COMPOSITED] presente_us_moy=", (i64)(pres_ns / trames_total / 1000));
                ecris_nombre("[COMPOSITED] presente_us_max=", (i64)(pres_max / 1000));
                ecris_nombre("[COMPOSITED] recompositions=", (i64)recompositions);
                ecris_nombre("[COMPOSITED] duree_ms=", (i64)((maintenant_ns() - debut_ns) / 1000000));
                for (int k = 0; k < nfen; k++) publie_trames(k);
            }
        }
        /* Scenario : la fenetre du fond glisse, puis monte au premier plan. */
        if (scenario && etape < 21 && nfen >= 2 && fens[0].visible && fens[1].visible
            && fens[0].trames >= 240 && fens[1].trames >= 240) {
            if (etape == 0) ecris("COMPOSITED_BUREAU_SCENARIO debut deplacement\n");
            if (etape < 20) {
                deplace(0, fens[0].x + 8, fens[0].y + 4);
            } else {
                remonte(0);
                /* TOUTES les fenetres visibles : `remonte` decale le rang de
                 * chacune. Ne republier que 0 et 1 laissait a une troisieme
                 * (la mire du banc applications) un rang perime, egal a celui
                 * de la fenetre remontee -- et le juge l'empilait au-dessus. */
                for (int k = 0; k < nfen; k++)
                    if (fens[k].visible) publie_fenetre(k, "COMPOSITED_BUREAU_SCENARIO_FIN fenetre=");
                ecris("COMPOSITED_BUREAU_SCENARIO_PILE_PUBLIEE\n");
            }
            etape++;
        }
        /* Scenario complet : une etape toutes les 4 s -- le banc capture
         * l'ecran entre deux etapes, et chaque capture doit tomber DANS son
         * etape (une cadence en trames les rapprochait a 1 s sous TCG). */
        if (scenario == 2 && etape >= 21 && etape < 27) {
            u64 maintenant = maintenant_ns();
            if (attente_etape == 0) attente_etape = maintenant + 4000000000UL;
            if (maintenant >= attente_etape) {
                attente_etape = 0;
                if (etape == 21) {
                    i64 z = sys6(BO_FENETRE_CONFIGURE, (u64)fens[1].poignee, 480, 300, 1, 0, 0);
                    exige(z == (((i64)480 << 32) | 300), "bureau : redimensionnement 480x300 accepte");
                    i64 trop = sys6(BO_FENETRE_CONFIGURE, (u64)fens[1].poignee, 0, 300, 1, 0, 0);
                    exige(trop == -BO_E_INVALID_ARGUMENT, "bureau : zone vide refusee");
                    ecris("COMPOSITED_BUREAU_ETAPE redimension fenetre=1\n");
                } else if (etape == 22) {
                    minimise(0);
                } else if (etape == 23) {
                    restaure(0);
                } else if (etape == 24) {
                    exige(sys6(BO_FENETRE_FERME, (u64)fens[1].poignee, 0, 0, 0, 0, 0) == 0,
                          "bureau : fermeture demandee");
                    ecris("COMPOSITED_BUREAU_ETAPE fermeture fenetre=1\n");
                } else if (etape == 25) {
                    if (fens[1].vivante) { attente_etape = maintenant; continue; }  /* attendre la sortie */
                    i64 h = sys6(BO_DISPLAY_RELANCE_CLIENT, (u64)affichage_g, 1, 0, 0, 0, 0);
                    exige(h > 0, "bureau : application relancee");
                    i64 hors = sys6(BO_DISPLAY_RELANCE_CLIENT, (u64)affichage_g, 7, 0, 0, 0, 0);
                    exige(hors < 0, "bureau : relance hors configuration refusee");
                    if (h > 0) {
                        struct fen *f = &fens[1];
                        f->poignee = h; f->vivante = 1; f->visible = 0; f->minimisee = 0; f->trames = 0; f->livrees = 0;
                        sys6(BO_WAITSET_ADD, (u64)attente, (u64)h, 2, 0, 0, 0);
                        vivantes++;
                        ecris("COMPOSITED_BUREAU_ETAPE relance fenetre=1\n");
                    }
                } else if (etape == 26) {
                    ecris(echecs == 0 ? "COMPOSITED_BUREAU_SCENARIO_COMPLET\n" : "COMPOSITED_BUREAU_SCENARIO_ECHEC\n");
                }
                etape++;
            }
        }
    }

    ecris_nombre("COMPOSITED_BUREAU_BILAN trames=", (i64)trames_total);
    for (int k = 0; k < nfen; k++) publie_trames(k);
    if (trames_total) {
        ecris_nombre("[COMPOSITED] lit_us_moy=", (i64)(lit_ns / trames_total / 1000));
        ecris_nombre("[COMPOSITED] presente_us_moy=", (i64)(pres_ns / trames_total / 1000));
    }
    ecris(echecs == 0 && trames_total > 0 ? "COMPOSITED_BUREAU_OK\n" : "COMPOSITED_BUREAU_FAIL\n");
    /* Le bureau reste affiche : le service vit, sans plus rien a composer. */
    for (;;) sys6(BO_WAITSET_WAIT, (u64)attente, (u64)evenements, FEN_MAX, 1000000000UL, 0, 0);
    return 0;
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
    if (affichage >= 0 && lit_fenetres(pile) > 0) {
        /* `/bin/composited bureau` : de vraies applications, d'autres processus. */
        nfen = 0;
        code = bureau(affichage, pile);
    } else if (affichage >= 0) {
        /* `/bin/composited presentation`, lance par le noyau. */
        code = presentation(affichage);
    } else {
        code = principal();
        if (presentation(-1) != 0) code = 1;
    }
    sys6(231, (u64)code, 0, 0, 0, 0, 0); /* exit_group */
    __builtin_unreachable();
}
