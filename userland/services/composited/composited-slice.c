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

#define BO_RIGHT_READ     (1u << 0)
#define BO_RIGHT_WRITE    (1u << 1)
#define BO_RIGHT_MAP      (1u << 3)
#define BO_RIGHT_DUP      (1u << 4)
#define BO_RIGHT_TRANSFER (1u << 5)
#define BO_RIGHT_INSPECT  (1u << 6)

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
    ecris(etiquette);
    ecris(&tampon[i + 1]);
    ecris("\n");
}

static int echecs = 0;

static void exige(int condition, const char *nom)
{
    if (condition) {
        ecris("[COMPOSITED] ok   ");
    } else {
        ecris("[COMPOSITED] ECHEC ");
        echecs++;
    }
    ecris(nom);
    ecris("\n");
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

void _start(void)
{
    int code = principal();
    sys6(231, (u64)code, 0, 0, 0, 0, 0); /* exit_group */
    __builtin_unreachable();
}
