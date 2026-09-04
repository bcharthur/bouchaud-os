/*
 * Format de fil de `composited`, cote C.
 *
 * Ce fichier DOIT rester d'accord avec `src/gui/composited.rs`, octet pour
 * octet. Rien dans la chaine de construction ne les relie -- le noyau se
 * compile pour `x86_64-bouchaud_os`, ce service pour la cible ring 3, et ils
 * ne se rencontrent qu'a l'execution, ou un desaccord d'un octet se manifeste
 * sous la forme d'une surface qui ne s'ouvre pas et d'aucun message.
 *
 * `tools/verifie-protocole-composited.py` est le lien manquant : il compare
 * les deux et echoue si l'un bouge sans l'autre.
 */
#ifndef BOUCHAUD_COMPOSITED_PROTOCOLE_H
#define BOUCHAUD_COMPOSITED_PROTOCOLE_H

#include <stdint.h>

/* "BOCO" */
#define COMPOSITED_MAGIC        0x4f434f42u
#define COMPOSITED_VERSION      1u
#define COMPOSITED_ENTETE       16u
#define COMPOSITED_CHARGE_MAX   4096u
#define COMPOSITED_TAMPONS      2u
#define COMPOSITED_SURFACES_MAX 32u

/* client -> composited */
#define COMPOSITED_DEMANDE_SURFACE  1u
#define COMPOSITED_TRAME_LIVREE     2u
#define COMPOSITED_DETACHE          3u
/* composited -> client */
#define COMPOSITED_SURFACE_ACCORDEE 0x100u
#define COMPOSITED_TAMPON_RENDU     0x101u
#define COMPOSITED_RECONFIGURE      0x102u
#define COMPOSITED_REFUS            0x103u

/* Raisons de refus. */
#define COMPOSITED_REFUS_PLUS_DE_SURFACE   1u
#define COMPOSITED_REFUS_GEOMETRIE         2u
#define COMPOSITED_REFUS_DEJA_ATTACHE      3u
#define COMPOSITED_REFUS_TAMPON_NON_POSSEDE 4u
#define COMPOSITED_REFUS_INCONNUE          5u

/* Echelle : cent-vingtiemes, comme `gui::protocole::ECHELLE_UNITE`. */
#define COMPOSITED_ECHELLE_UNITE 120u

#define COMPOSITED_TAILLE_SURFACE_ACCORDEE 32u
#define COMPOSITED_TAILLE_TRAME_LIVREE     28u
#define COMPOSITED_TAILLE_TAMPON_RENDU     16u

static inline void co_pose_u32(unsigned char *o, unsigned d, uint32_t v)
{
    o[d] = (unsigned char)(v & 0xFF);
    o[d + 1] = (unsigned char)((v >> 8) & 0xFF);
    o[d + 2] = (unsigned char)((v >> 16) & 0xFF);
    o[d + 3] = (unsigned char)((v >> 24) & 0xFF);
}

static inline uint32_t co_lit_u32(const unsigned char *o, unsigned d)
{
    return (uint32_t)o[d] | ((uint32_t)o[d + 1] << 8)
         | ((uint32_t)o[d + 2] << 16) | ((uint32_t)o[d + 3] << 24);
}

static inline void co_pose_entete(unsigned char *o, uint16_t genre,
                                  uint32_t taille, uint32_t serie)
{
    co_pose_u32(o, 0, COMPOSITED_MAGIC);
    o[4] = (unsigned char)(COMPOSITED_VERSION & 0xFF);
    o[5] = (unsigned char)((COMPOSITED_VERSION >> 8) & 0xFF);
    o[6] = (unsigned char)(genre & 0xFF);
    o[7] = (unsigned char)((genre >> 8) & 0xFF);
    co_pose_u32(o, 8, taille);
    co_pose_u32(o, 12, serie);
}

/* Rend 0 si l'en-tete n'est pas le notre. */
static inline int co_entete_valide(const unsigned char *o)
{
    uint32_t magic = co_lit_u32(o, 0);
    uint16_t version = (uint16_t)((uint16_t)o[4] | ((uint16_t)o[5] << 8));
    return magic == COMPOSITED_MAGIC && version == COMPOSITED_VERSION;
}

static inline uint16_t co_genre(const unsigned char *o)
{
    return (uint16_t)((uint16_t)o[6] | ((uint16_t)o[7] << 8));
}

#endif /* BOUCHAUD_COMPOSITED_PROTOCOLE_H */
