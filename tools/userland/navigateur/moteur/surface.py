"""La surface partagee entre le navigateur et son renderer.

## Pourquoi la memoire partagee, et pas le canal

Un `socketpair` a un tampon borne — 64 KiB sous Bouchaud OS. Y faire passer une
fenetre de 1280x720 en 32 bits, c'est 3,5 Mio a decouper en cinquante-six
morceaux, a recopier deux fois — une a l'ecriture, une a la lecture — et a
faire transiter pendant que le canal de controle attend son tour.

Avec `memfd_create` + `mmap(MAP_SHARED)`, les deux processus ecrivent dans les
**memes frames physiques**. Il n'y a plus de copie du tout, et le canal de
controle reste libre pour ce qu'il sait faire : de petits messages, tout de
suite.

## Deux tampons, et pourquoi deux

Un seul tampon obligerait le renderer a attendre que le navigateur ait fini de
peindre pour commencer la trame suivante — ou, s'il n'attendait pas, a ecrire
dans ce que l'autre est en train de lire. On voit alors la moitie d'une trame
et la moitie de la suivante : ce qu'un joueur appelle une dechirure.

Deux tampons : le renderer peint dans celui que le navigateur ne lit pas, puis
les echange. L'echange n'est qu'un entier a ecrire dans l'en-tete.

## Ce qui synchronise vraiment

Pas l'en-tete — un entier partage entre deux processus ne prouve rien tout
seul. C'est le message `FRAME_READY` du canal de controle qui fait barriere :
le renderer finit d'ecrire ses pixels, **puis** publie la generation dans
l'en-tete, **puis** envoie le message. Le navigateur ne lit qu'apres avoir recu
le message, donc apres tout ce qui precede. L'ordre des trois etapes est la
seule chose qui compte, et il est ecrit une fois, ici.

## La disposition

    +----------------------------------+  offset 0
    | en-tete : 64 octets              |
    +----------------------------------+  offset 64
    | tampon 0 : hauteur * pas octets  |
    +----------------------------------+
    | tampon 1 : hauteur * pas octets  |
    +----------------------------------+

L'en-tete est plus grand qu'il n'a besoin de l'etre. C'est deliberement de la
place laissee : ajouter un champ un jour ne doit pas decaler les pixels, ce qui
obligerait les deux cotes a changer en meme temps.
"""

import mmap
import os
import struct

MAGIC = 0x424F5346          # "BOSF"
VERSION = 1

# magic, version, largeur, hauteur, pas, generation, avant, reserve
EN_TETE = struct.Struct("<IIIIIIII")
TAILLE_EN_TETE = 64
assert EN_TETE.size <= TAILLE_EN_TETE

OCTETS_PAR_PIXEL = 4
TAMPONS = 2

# Une surface plus grande que cela n'est pas une fenetre, c'est une erreur de
# calcul. La borne protege les deux cotes : celui qui alloue comme celui qui
# recoit un descripteur et lit son en-tete.
PIXELS_MAX = 8192 * 8192


def taille_pour(largeur, hauteur):
    pas = largeur * OCTETS_PAR_PIXEL
    return TAILLE_EN_TETE + TAMPONS * hauteur * pas


class Surface:
    """Un `memfd` mappe, vu des deux cotes de la meme facon."""

    def __init__(self, descripteur, largeur, hauteur, proprietaire=False):
        if largeur <= 0 or hauteur <= 0 or largeur * hauteur > PIXELS_MAX:
            raise ValueError("dimensions de surface invalides : %sx%s"
                             % (largeur, hauteur))
        self.descripteur = descripteur
        self.largeur = int(largeur)
        self.hauteur = int(hauteur)
        self.pas = self.largeur * OCTETS_PAR_PIXEL
        self.octets_tampon = self.hauteur * self.pas
        self.taille = taille_pour(self.largeur, self.hauteur)
        self.carte = mmap.mmap(descripteur, self.taille,
                               flags=mmap.MAP_SHARED,
                               prot=mmap.PROT_READ | mmap.PROT_WRITE)
        if proprietaire:
            self._ecris_entete(0, 0)

    @classmethod
    def alloue(cls, largeur, hauteur, nom="bo-surface"):
        """Cree la surface. **Le navigateur seul le fait.**

        Ce n'est pas un detail d'organisation : un renderer qui allouerait ses
        propres surfaces pourrait en allouer mille, et `RLIMIT_AS` le tuerait —
        ce qui est correct mais brutal, et surtout impossible a distinguer d'un
        vrai defaut. La surface appartient a celui qui tient la fenetre.
        """
        if largeur <= 0 or hauteur <= 0 or largeur * hauteur > PIXELS_MAX:
            raise ValueError("dimensions de surface invalides : %sx%s"
                             % (largeur, hauteur))
        descripteur = os.memfd_create(nom, 0)
        os.ftruncate(descripteur, taille_pour(largeur, hauteur))
        return cls(descripteur, largeur, hauteur, proprietaire=True)

    # --- En-tete --------------------------------------------------------------

    def _ecris_entete(self, generation, avant):
        self.carte[0:EN_TETE.size] = EN_TETE.pack(
            MAGIC, VERSION, self.largeur, self.hauteur, self.pas,
            generation, avant, 0)

    def entete(self):
        (magic, version, largeur, hauteur, pas, generation, avant,
         _reserve) = EN_TETE.unpack(self.carte[0:EN_TETE.size])
        if magic != MAGIC or version != VERSION:
            raise ValueError("en-tete de surface illisible")
        return {"largeur": largeur, "hauteur": hauteur, "pas": pas,
                "generation": generation, "avant": avant}

    # --- Pixels ---------------------------------------------------------------

    def _debut(self, index):
        return TAILLE_EN_TETE + index * self.octets_tampon

    def ecris(self, index, octets):
        """Depose une trame dans le tampon `index`, sans publier.

        Publier est un acte separe (`publie`), et l'ordre des deux est ce qui
        empeche l'autre cote de voir une trame a moitie ecrite.
        """
        if len(octets) != self.octets_tampon:
            raise ValueError("trame de %d octets, attendu %d"
                             % (len(octets), self.octets_tampon))
        debut = self._debut(index)
        self.carte[debut:debut + self.octets_tampon] = octets

    def publie(self, index, generation):
        """Declare le tampon `index` lisible. A faire **apres** l'avoir ecrit."""
        self._ecris_entete(generation, index)

    def lis(self, index=None):
        """Rend les octets du tampon publie, ou de celui qu'on nomme."""
        if index is None:
            index = self.entete()["avant"]
        debut = self._debut(index)
        return bytes(self.carte[debut:debut + self.octets_tampon])

    def pixel(self, x, y, index=None):
        """Un pixel, en `(b, g, r, a)` — l'ordre de la memoire, pas du nom."""
        if index is None:
            index = self.entete()["avant"]
        debut = self._debut(index) + y * self.pas + x * OCTETS_PAR_PIXEL
        return tuple(self.carte[debut:debut + OCTETS_PAR_PIXEL])

    def ferme(self):
        try:
            self.carte.close()
        except (BufferError, ValueError):
            pass
        if self.descripteur is not None:
            try:
                os.close(self.descripteur)
            except OSError:
                pass
            self.descripteur = None
