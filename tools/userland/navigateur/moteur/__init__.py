"""Moteur web natif de Bouchaud OS.

Enchainement, dans l'ordre :

    reseau      URL   -> octets
    html        octets -> arbre de nœuds
    css         feuilles + attributs `style` -> regles
    js          scripts de la page -> modifications de l'arbre
    mise_en_page arbre + regles -> boites positionnees
    peinture    boites -> liste d'affichage

L'hote Qt (`hote.cpp`) n'intervient qu'aux deux extremites : il fournit la
mesure du texte a la mise en page, le decodage des images, et il peint la liste
d'affichage. Tout ce qu'il y a entre les deux est du Python, et se teste sans
ecran.

## Le JavaScript, et quand il tourne

Les scripts s'executent une fois l'arbre construit, avant la premiere mise en
page : c'est ce qui permet a une page qui fabrique son contenu dans
`DOMContentLoaded` de s'afficher. Ensuite, le document se remet en page des que
le script touche a l'arbre — voir [`Document.rafraichis`], que le navigateur
appelle a son battement.

Une page sans script ne paie rien : le contexte JavaScript n'est cree que si
elle en contient au moins un.
"""

import time
import urllib.parse

import bo

from . import (animation, css, edition, html, images, mise_en_page,
               peinture, police, prechargement, reseau, stockage,
               telemetrie)

__all__ = ["animation", "css", "html", "images", "js", "mise_en_page",
           "peinture", "police", "prechargement", "reseau", "stockage",
           "Document"]


class Document:
    """Une page chargee : son arbre, ses regles, ses scripts, sa mise en page."""

    def __init__(self, reponse, largeur, scripts=True, journal=None,
                 hauteur_fenetre=720.0, precharge=True):
        self.url = reponse.url
        self.code = reponse.code
        self.erreur = reponse.erreur
        with telemetrie.chrono("html"):
            self.racine = html.analyse(reponse.contenu)
        self.titre = self._titre()
        self.journal = journal or (lambda niveau, texte: None)
        # Feuilles liees deja rapportees, par adresse : la cascade est
        # rejouee a chaque redimensionnement, pas le telechargement.
        self._feuilles = {}
        # Index des regles, et signature des feuilles dont il est issu.
        self._index = None
        self._signature = None
        # Dernier element survole, retenu pour que `:active` et `:hover` se
        # composent au lieu de s'effacer l'un l'autre.
        self._survole = None
        # Ce qui bouge : gabarits `@keyframes`, animations en cours,
        # transitions engagees. Neuf a chaque page, donc rien ne survit a une
        # navigation.
        self.animateur = animation.Animateur()
        # Polices deja demandees, par (famille, gras, italique). Une page qui se
        # remet en page dix fois ne les retelecharge pas.
        self._polices = {}

        # Les sous-ressources partent maintenant, ensemble, pendant qu'on
        # analyse les feuilles : quand la mise en page reclamera une image, elle
        # sera deja la. C'est le seul endroit ou l'ordre compte — precharger
        # apres la mise en page ne servirait plus a rien.
        if precharge:
            prechargement.precharge(self.racine, self.url, self.journal)

        # La taille de la fenetre doit etre connue **avant** l'analyse des
        # feuilles : c'est elle qui decide quelles regles `@media` sont
        # retenues, et `vw`/`vh` s'y rapportent.
        self.hauteur_fenetre = float(hauteur_fenetre) or 720.0
        css.pose_fenetre(largeur, self.hauteur_fenetre)
        self.regles = self._regles()
        self.largeur = largeur
        self.boite = None
        self.hauteur = 0.0
        self.zones_liens = []
        self.contexte_js = None
        # L'element au foyer. Un seul par document, comme le veut la norme.
        self._foyer = None
        # L'etat d'edition de chaque champ touche, par identite d'element.
        self._editions = {}
        # Vrai quand le curseur a bouge sans que la valeur change : il faut
        # repeindre, pas remettre en page.
        self.sale_curseur = False

        if scripts:
            self._demarre_scripts()
        self.remet_en_page(largeur)
        if self.contexte_js is not None:
            # `load` part apres la premiere mise en page : c'est la que
            # `getBoundingClientRect` a enfin des valeurs a rendre.
            self.contexte_js.signale_pret()
            self.rafraichis()

    # --- Scripts --------------------------------------------------------------

    def _a_des_scripts(self):
        for element in self.racine.parcours():
            if isinstance(element, html.Element) and element.balise == "script":
                return True
        return False

    def _demarre_scripts(self):
        if not self._a_des_scripts():
            return
        try:
            from . import js
        except ImportError as e:  # hote sans QuickJS : la page reste statique
            self.journal("warn", "JavaScript indisponible : %s" % e)
            return
        try:
            self.contexte_js = js.Contexte(self, journal=self.journal)
            self.contexte_js.execute_scripts()
        except Exception as e:  # noqa: BLE001 — un script ne tue pas la page
            self.journal("error", "JavaScript : %s" % e)
            self.contexte_js = None
        # L'element au foyer. Un seul par document, comme le veut la norme.
        self._foyer = None
        # L'etat d'edition de chaque champ touche, par identite d'element.
        self._editions = {}
        # Vrai quand le curseur a bouge sans que la valeur change : il faut
        # repeindre, pas remettre en page.
        self.sale_curseur = False

    def rafraichis(self):
        """Remet en page si le JavaScript a touche a l'arbre. Rend `True` alors.

        A appeler au battement du navigateur : c'est ce qui fait qu'un
        `setTimeout` qui change le DOM se voit reellement a l'ecran.
        """
        self.animateur.pose_temps(time.monotonic() * 1000.0)

        sale = False
        contexte = self.contexte_js
        if contexte is not None:
            contexte.tic()
            if contexte.sale:
                contexte.sale = False
                # Les regles peuvent avoir change : un script qui insere un
                # `<style>` est un cas courant des bibliotheques de composants.
                self.regles = self._regles()
                self.titre = self.titre or self._titre()
                sale = True

        # Une animation en cours redemande une mise en page a chaque battement :
        # c'est elle qui fait avancer le mouvement. Quand plus rien ne bouge,
        # l'animateur baisse son drapeau et le navigateur cesse de redessiner.
        if not sale and not self.animateur.anime():
            return False
        self.remet_en_page(self.largeur)
        return True

    def evenement_js(self, nœud, type_, details=None):
        """Distribue un evenement d'interface dans la page. Rend `False` si le
        script a demande d'annuler l'action par defaut (`preventDefault`)."""
        if self.contexte_js is None:
            return True
        resultat = self.contexte_js.evenement(nœud, type_, details)
        self.rafraichis()
        return resultat is not False

    @property
    def navigation_demandee(self):
        """URL que le script a demandee par `location.href = …`, ou `None`."""
        if self.contexte_js is None:
            return None
        demande = self.contexte_js.navigation
        self.contexte_js.navigation = None
        return demande

    def ferme(self):
        if self.contexte_js is not None:
            self.contexte_js.ferme()
            self.contexte_js = None
        # L'element au foyer. Un seul par document, comme le veut la norme.
        self._foyer = None
        # L'etat d'edition de chaque champ touche, par identite d'element.
        self._editions = {}
        # Vrai quand le curseur a bouge sans que la valeur change : il faut
        # repeindre, pas remettre en page.
        self.sale_curseur = False

    # --- Arbre et mise en page ------------------------------------------------

    def _titre(self):
        element = self.racine.trouve("title")
        if element:
            titre = " ".join(element.texte().split())
            if titre:
                return titre
        return self.url

    def _regles(self):
        with telemetrie.chrono("css"):
            return self._regles_reellement()

    def _regles_reellement(self):
        """Cascade complete : feuille de l'agent, feuilles liees, feuilles en ligne.

        L'ordre du document decide entre deux regles de meme specificite ; un
        `<link>` et un `<style>` sont donc numerotes dans l'ordre ou ils
        apparaissent, et non par categorie. Une feuille liee qui suit un
        `<style>` doit pouvoir le contredire, comme dans tout navigateur.
        """
        # Chaque feuille est retenue avec l'ombre d'ou elle vient : une feuille
        # posee dans une racine d'ombre ne doit styler que ce qu'elle contient.
        sources = []
        ombres = False
        for element in self.racine.parcours():
            if not isinstance(element, html.Element):
                continue
            if element.balise == css.SUPPORT_OMBRE:
                ombres = True
                continue
            if element.balise == "style":
                sources.append((element.texte(), css.ombre_de(element), self.url))
            elif element.balise == "link":
                sources.append((self._feuille_liee(element), None,
                                self._adresse_feuille(element)))
        # A poser avant toute cascade : c'est ce drapeau qui decide si la
        # frontiere est calculee ou sautee.
        css.pose_ombres(ombres)
        if ombres:
            # Les ombres ont pu apparaitre apres la premiere lecture des
            # feuilles : `ombre_de` ne repondait juste qu'une fois le drapeau
            # leve, donc on refait la passe.
            sources = []
            for element in self.racine.parcours():
                if not isinstance(element, html.Element):
                    continue
                if element.balise == "style":
                    sources.append((element.texte(), css.ombre_de(element),
                                    self.url))
                elif element.balise == "link":
                    sources.append((self._feuille_liee(element), None,
                                    self._adresse_feuille(element)))

        # Analyser une feuille de deux mille regles a chaque battement du
        # JavaScript coute plus que toute la mise en page. Tant que les feuilles
        # sont les memes — texte compris, car un script peut reecrire un
        # `<style>` — on garde l'index deja construit.
        signature = tuple((texte, id(ombre)) for texte, ombre, _ in sources)
        if signature == self._signature and self._index is not None:
            return self._index

        keyframes = {}
        polices = []
        regles = css.analyse(css.FEUILLE_PAR_DEFAUT, keyframes=keyframes,
                             ombre=css.PORTEE_AGENT)
        ordre = len(regles) + 1000
        for source, ombre, base in sources:
            if not source:
                continue
            declarees = []
            nouvelles = css.analyse(source, ordre, keyframes=keyframes, ombre=ombre,
                                    polices=declarees)
            regles.extend(nouvelles)
            ordre += len(nouvelles) + 1
            # Une police se resout contre la feuille qui la cite, non contre la
            # page : `url(../webfonts/x.woff)` d'une feuille rangee dans
            # `/static/css/` ne designe pas le meme fichier vu de la racine.
            polices.extend((declaration, base) for declaration in declarees)
        self.animateur.keyframes = keyframes
        self._charge_polices(polices)

        self._signature = signature
        self._index = css.indexe(regles)
        return self._index

    def _charge_polices(self, declarations):
        """Telecharge les `@font-face` de la page et les remet a l'hote.

        Sans elles, un site s'affiche dans la police du systeme : passe encore
        pour du texte, fatal pour une police d'icones, dont les glyphes vivent
        dans une zone privee d'Unicode et sortent en carres.

        Une police n'est demandee qu'une fois par adresse, et une famille deja
        chargee n'est pas rechargee : la cascade est rejouee a chaque
        redimensionnement.
        """
        remet = getattr(bo, "police", None)
        if remet is None or not declarations:
            return
        for declaration, base in declarations:
            noms = css.familles(declaration.get("font-family", ""))
            if not noms:
                continue
            famille = noms[0]
            gras = _poids_gras(declaration.get("font-weight", "400"))
            italique = declaration.get("font-style", "normal").strip() == "italic"
            cle = (famille.lower(), gras, italique)
            if cle in self._polices:
                continue
            # Une coupe qui n'ecrit pas de latin n'est pas retenue : plusieurs
            # `@font-face` partagent la meme famille, chacun pour une ecriture,
            # et garder le dernier ferait sortir toute la page en carres.
            if not police.couvre_latin(declaration.get("unicode-range", "")):
                continue

            choix = police.meilleure_source(declaration.get("src", ""))
            if choix is None:
                self._polices[cle] = False
                self.journal("info", "police %s : aucun format lisible (WOFF2 seul)"
                             % famille)
                continue
            url = urllib.parse.urljoin(base or self.url or "", choix[0])
            reponse = reseau.charge(url, brut=True, document=self.url,
                                    destination="font")
            ouverte = police.ouvre(reponse.octets or b"") if reponse.code == 200 else None
            if not ouverte:
                if not reponse.code or reponse.code >= 400:
                    telemetrie.ressource_echouee(url, reponse.code, "font")
                self._polices[cle] = False
                self.journal("warn", "police %s : %s illisible" % (famille, choix[1]))
                continue
            try:
                pose = bool(remet(ouverte, famille, gras, italique))
            except Exception as e:  # noqa: BLE001
                self.journal("warn", "police %s : %s" % (famille, e))
                pose = False
            self._polices[cle] = pose
            if pose:
                police.retiens(famille)

    def _adresse_feuille(self, element):
        """L'adresse absolue d'un `<link rel=stylesheet>`, ou celle de la page."""
        adresse = (element.attributs.get("href") or "").strip()
        if not adresse:
            return self.url
        return urllib.parse.urljoin(self.url or "", adresse)

    def _feuille_liee(self, element):
        """Le texte d'un `<link rel="stylesheet">`, ou `""`.

        Le prechargement a en principe deja rapporte le fichier ; sinon on le
        demande ici. Une feuille absente ne fait pas echouer la page : elle
        s'affiche avec les regles qu'on a, ce que fait aussi un navigateur quand
        une requete echoue.
        """
        rel = element.attributs.get("rel", "").lower().split()
        if "stylesheet" not in rel:
            return ""
        adresse = (element.attributs.get("href") or "").strip()
        if not adresse:
            return ""
        media = element.attributs.get("media", "")
        if media and not css.requete_verifiee(media):
            return ""

        url = urllib.parse.urljoin(self.url or "", adresse)
        cle = url
        if cle in self._feuilles:
            return self._feuilles[cle]
        try:
            reponse = reseau.charge(url, brut=True, document=self.url,
                                    destination="style")
        except Exception as e:  # noqa: BLE001
            telemetrie.ressource_echouee(url, 0, "stylesheet")
            self.journal("warn", "feuille %s : %s" % (adresse, e))
            self._feuilles[cle] = ""
            return ""
        if not reponse.code or reponse.code >= 400:
            # Une feuille perdue, c'est une page qui s'affiche nue. Le rapport
            # doit le dire aussi fort qu'un script perdu.
            telemetrie.ressource_echouee(url, reponse.code, "stylesheet")
        texte = "" if (reponse.code and reponse.code >= 400) else reponse.contenu
        self._feuilles[cle] = texte
        return texte

    def remet_en_page(self, largeur, hauteur_fenetre=None):
        self.largeur = largeur
        if hauteur_fenetre:
            self.hauteur_fenetre = float(hauteur_fenetre)
        # Une fenetre redimensionnee change les `@media` retenues : les regles
        # sont donc reanalysees, pas seulement reappliquees.
        ancienne = css.fenetre()
        css.pose_fenetre(largeur, self.hauteur_fenetre)
        if ancienne != css.fenetre():
            # Les feuilles n'ont pas change, mais les `@media` retenues si :
            # l'index memorise ne vaut plus, il faut le reconstruire.
            self._signature = None
            self.regles = self._regles()
        # L'animateur n'est pose que le temps de la mise en page : hors de la,
        # un `getComputedStyle` appele par un script rejouerait la cascade et
        # ferait croire a une valeur changee, ce qui declencherait des
        # transitions que la page n'a pas demandees.
        self.animateur.nouveau_tour()
        css.pose_animateur(self.animateur)
        try:
            with telemetrie.chrono("mise_en_page"):
                self.boite, self.hauteur = mise_en_page.construit(
                    self.racine, self.regles, largeur, self.url,
                    self._image_video, self._toile)
            self.pose_curseur_visuel()
        finally:
            css.pose_animateur(None)

    def _toile(self, nœud):
        """Ce qu'un `<canvas>` a dessine, ou `None` s'il n'a rien dessine."""
        if self.contexte_js is None:
            return None
        return self.contexte_js.toile(nœud)

    def _image_video(self, nœud):
        """Image courante d'un `<video>`, ou `None` s'il n'y en a pas encore."""
        if self.contexte_js is None:
            return None
        return self.contexte_js.image_video(nœud)

    def liste_affichage(self, defilement, largeur_vue, hauteur_vue):
        self.zones_liens = []
        with telemetrie.chrono("peinture"):
            return peinture.peint(self.boite, defilement, largeur_vue,
                                  hauteur_vue, self.zones_liens)

    def lien_a(self, x, y):
        """URL du lien sous ce point, en coordonnees de page. `None` sinon."""
        for zx, zy, zl, zh, url in self.zones_liens:
            if zx <= x <= zx + zl and zy <= y <= zy + zh:
                return url
        return None

    def survole(self, x, y):
        """Declare la position du pointeur. Rend `True` si l'affichage change.

        `:hover` designe l'element **et toute sa lignee** : c'est ce qui permet
        a un menu deroulant de rester ouvert quand la souris quitte son bouton
        pour sa liste.

        Une page dont aucune regle ne parle d'interaction n'est pas recalculee —
        et c'est l'immense majorite des mouvements de souris. Sans ce
        court-circuit, bouger le pointeur relancerait la cascade complete a
        chaque pixel.
        """
        return self._interaction(survoles=self.element_a(x, y))

    def enfonce(self, x, y):
        """Declare le bouton enfonce sous ce point (`:active`)."""
        cible = self.element_a(x, y) if x is not None else None
        return self._interaction(survoles=self._survole, actifs=cible)

    def relache(self):
        """Le bouton n'est plus enfonce."""
        return self._interaction(survoles=self._survole, actifs=None)

    # --- Foyer ----------------------------------------------------------------
    #
    # Le foyer appartient au **document**, pas au JavaScript ni au chrome. Les
    # deux le deplacent — un clic reel d'un cote, un `element.focus()` de
    # l'autre — et les deux doivent lire le meme. Le mettre ailleurs qu'ici
    # aurait donne deux verites, donc un `document.activeElement` qui contredit
    # ce que l'utilisateur voit surligne.

    def foyer_actuel(self):
        """L'element au foyer, ou `None`."""
        return self._foyer

    # --- Edition de texte -----------------------------------------------------

    def edition_de(self, element, cree=True):
        """L'etat d'edition d'un champ : valeur courante, curseur, selection.

        Cree a la premiere demande, a partir de l'attribut `value` — qui reste
        la valeur **par defaut**, celle que `form.reset()` restaure. La valeur
        courante vit ici. Les confondre, comme avant, rendait `reset()`
        incapable de restaurer quoi que ce soit puisqu'il avait lui-meme ecrase
        ce qu'il devait remettre.
        """
        if element is None or not edition.est_editable(element):
            return None
        etat = self._editions.get(id(element))
        if etat is None and cree:
            depart = (element.texte() if element.balise == "textarea"
                      else element.attributs.get("value", ""))
            etat = edition.Edition(depart,
                                   multiligne=edition.multiligne(element),
                                   masque=edition.est_masque(element))
            self._editions[id(element)] = etat
        return etat

    def frappe(self, touche, texte="", maj=False, ctrl=False):
        """Une touche reelle, adressee a l'element au foyer.

        L'ordre des evenements est celui du Web, et il compte : `keydown`
        d'abord — la page peut annuler et rien ne sera insere —, la valeur
        ensuite, `input` apres la valeur, `keyup` en dernier. Emettre `input`
        avant la mutation, ou `keydown` apres, ferait lire a la page un etat qui
        n'existe pas encore.

        Rend `True` si la valeur a change.
        """
        cible = self._foyer
        contexte = self.contexte_js
        details = {"key": touche, "shiftKey": bool(maj), "ctrlKey": bool(ctrl)}

        if contexte is not None and cible is not None:
            # `keydown` est annulable : `e.preventDefault()` dans un `keydown`
            # doit empecher l'insertion, et c'est la forme la plus repandue de
            # filtrage de saisie du Web.
            if contexte.evenement(cible, "keydown", details) is False:
                contexte.evenement(cible, "keyup", details)
                return False

        etat = self.edition_de(cible)
        change = bouge = False
        if etat is not None:
            change, bouge = edition.applique_touche(etat, touche, texte,
                                                    maj=maj, ctrl=ctrl)
            if change:
                self._synchronise_champ(cible, etat)

        if touche == "Tab":
            self.deplace_foyer(-1 if maj else 1)
            bouge = True
        elif touche == "Enter" and etat is not None and not etat.multiligne:
            self._entree_soumet(cible)
        elif touche in (" ", "Spacebar") and etat is None:
            # Sur une case ou un bouton, l'espace agit comme un clic. C'est le
            # seul moyen de cocher au clavier.
            self.active(cible)

        if contexte is not None and cible is not None:
            if change:
                contexte.evenement(cible, "input", details)
            contexte.evenement(cible, "keyup", details)

        if change or bouge:
            self.sale_curseur = True
        if change:
            self.remet_en_page(self.largeur)
        return change

    def _synchronise_champ(self, element, etat):
        """Reporte la valeur courante la ou la mise en page la lira."""
        element.valeur_courante = etat.valeur

    def _entree_soumet(self, champ):
        """`Entree` dans un champ envoie le formulaire qui le contient."""
        contexte = self.contexte_js
        if contexte is None:
            return
        formulaire = champ
        while formulaire is not None and getattr(formulaire, "balise", None) != "form":
            formulaire = getattr(formulaire, "parent", None)
        if formulaire is None:
            return
        contexte.appel("soumets", contexte._identifiant(formulaire), True)

    def active(self, element):
        """L'action par defaut d'un clic ou d'un espace : cocher, choisir, envoyer.

        Ces trois-la n'ont rien d'accessoire : sans elles, une case ne se coche
        pas, un bouton n'envoie rien, et un formulaire n'est qu'un dessin.
        """
        if element is None or "disabled" in getattr(element, "attributs", {}):
            return False
        contexte = self.contexte_js
        balise = getattr(element, "balise", None)
        type_ = (element.attributs.get("type") or "").lower() if balise == "input" else ""

        if balise == "input" and type_ in ("checkbox", "radio"):
            if type_ == "radio":
                # Un seul bouton coche par nom : c'est la definition meme d'un
                # groupe radio, et sans cela on peut tout cocher a la fois.
                nom = element.attributs.get("name")
                if nom:
                    for autre in self.racine.parcours():
                        if (isinstance(autre, html.Element)
                                and autre is not element
                                and autre.balise == "input"
                                and (autre.attributs.get("type") or "").lower() == "radio"
                                and autre.attributs.get("name") == nom):
                            autre.attributs.pop("checked", None)
                element.attributs["checked"] = ""
            elif "checked" in element.attributs:
                element.attributs.pop("checked")
            else:
                element.attributs["checked"] = ""
            if contexte is not None:
                contexte.evenement(element, "input", {})
                contexte.evenement(element, "change", {})
            self.remet_en_page(self.largeur)
            return True

        if (balise == "button" and (element.attributs.get("type") or "submit").lower()
                == "submit") or (balise == "input" and type_ == "submit"):
            formulaire = element
            while formulaire is not None \
                    and getattr(formulaire, "balise", None) != "form":
                formulaire = getattr(formulaire, "parent", None)
            if formulaire is not None and contexte is not None:
                contexte.appel("soumets", contexte._identifiant(formulaire), True)
                return True
        return False

    def clique(self, x, y):
        """Un clic reel : trouve la cible, deplace le foyer, pose le curseur.

        C'est le chemin qui manquait entierement. Le foyer ne se deplacait que
        par `element.focus()` — donc jamais dans l'usage normal, ou l'on clique.
        """
        cible = self.element_a(x, y)
        focalisable = cible
        while focalisable is not None:
            if isinstance(focalisable, html.Element):
                balise = focalisable.balise
                if (balise in ("input", "select", "textarea", "button")
                        or (balise in ("a", "area") and "href" in focalisable.attributs)
                        or "tabindex" in focalisable.attributs):
                    break
                if balise == "label":
                    # Cliquer une etiquette donne le foyer au controle qu'elle
                    # designe : c'est ce que tout formulaire suppose, et sans
                    # cela la moitie des cases a cocher sont inatteignables.
                    designe = self._controle_de_label(focalisable)
                    if designe is not None:
                        focalisable = designe
                        break
            focalisable = getattr(focalisable, "parent", None)

        if focalisable is None or "disabled" in getattr(focalisable, "attributs", {}):
            # Cliquer dans le vide retire le foyer, comme dans un navigateur.
            self.pose_foyer(None)
            return None

        self.pose_foyer(focalisable)
        etat = self.edition_de(focalisable)
        if etat is not None:
            etat.pose_curseur(self._curseur_a(focalisable, etat, x))
            self.sale_curseur = True
        return focalisable

    def _controle_de_label(self, etiquette):
        cible = etiquette.attributs.get("for")
        if cible:
            for nœud in self.racine.parcours():
                if isinstance(nœud, html.Element) \
                        and nœud.attributs.get("id") == cible:
                    return nœud
            return None
        for nœud in etiquette.parcours():
            if isinstance(nœud, html.Element) \
                    and nœud.balise in ("input", "select", "textarea", "button"):
                return nœud
        return None

    def _curseur_a(self, element, etat, x):
        """L'indice de caractere le plus proche du point clique.

        On mesure les prefixes successifs et on prend celui dont le bord est le
        plus proche : c'est exact au caractere pres, et la mesure passe par
        l'hote, donc par la vraie fonte — la seule facon d'etre juste avec une
        police proportionnelle.
        """
        boite = self._boite_de(element)
        if boite is None:
            return len(etat.valeur)
        style = boite.style
        taille = mise_en_page._taille_police(style)
        gras = mise_en_page._est_gras(style)
        fixe = mise_en_page._est_fixe(style)
        famille = mise_en_page.famille(style)
        # Le texte commence apres la bordure et le remplissage gauche : mesurer
        # depuis le bord de la boite decalerait le curseur d'autant.
        depart = boite.x + (css.longueur(style.get("padding-left"), 0.0, taille) or 0.0) \
            + (css.longueur(style.get("border-left-width"), 0.0, taille) or 0.0)
        affiche = etat.texte_affiche()

        meilleur, ecart_min = 0, abs(x - depart)
        for index in range(1, len(affiche) + 1):
            largeur = bo.largeur_texte(affiche[:index], taille, gras, fixe, famille)
            ecart = abs(x - (depart + largeur))
            if ecart < ecart_min:
                meilleur, ecart_min = index, ecart
        return meilleur

    def pose_curseur_visuel(self):
        """Place le trait du curseur dans la boite du champ au foyer.

        Apres la mise en page, pas pendant : la position du curseur depend de
        la largeur du texte qui le precede, donc de la boite finale. La calculer
        en cours de disposition obligerait a la refaire des que la boite bouge.

        Le curseur n'est pas un caractere. Il est pose a cote du texte, dans un
        champ de la boite que la peinture lit. L'inserer dans la valeur — ce que
        fait la barre d'adresse du chrome — partirait avec le formulaire et
        changerait la largeur mesuree a chaque clignotement.
        """
        cible = self._foyer
        etat = self.edition_de(cible, cree=False) if cible is not None else None
        if etat is None:
            return False
        boite = self._boite_de(cible)
        if boite is None:
            return False
        style = boite.style
        taille = mise_en_page._taille_police(style)
        gras = mise_en_page._est_gras(style)
        fixe = mise_en_page._est_fixe(style)
        famille = mise_en_page.famille(style)
        gauche = boite.x \
            + (css.longueur(style.get("padding-left"), 0.0, taille) or 0.0) \
            + (css.longueur(style.get("border-left-width"), 0.0, taille) or 0.0)
        haut = boite.y \
            + (css.longueur(style.get("padding-top"), 0.0, taille) or 0.0) \
            + (css.longueur(style.get("border-top-width"), 0.0, taille) or 0.0)
        hauteur = bo.hauteur_ligne(taille, fixe)
        affiche = etat.texte_affiche()

        def largeur(jusqua):
            if jusqua <= 0:
                return 0.0
            return bo.largeur_texte(affiche[:jusqua], taille, gras, fixe, famille)

        debut, fin = etat.selection()
        selection = None
        if debut != fin:
            x_debut = largeur(debut)
            selection = (gauche + x_debut, largeur(fin) - x_debut)
        boite.curseur = (gauche + largeur(etat.curseur), haut, hauteur, selection)
        return True

    def _boite_de(self, element):
        pile = [self.boite] if self.boite is not None else []
        while pile:
            boite = pile.pop()
            if boite.element is element:
                return boite
            pile.extend(getattr(boite, "enfants", ()) or ())
        return None

    def pose_foyer(self, element, notifie=True):
        """Deplace le foyer. Rend `True` si quelque chose a change.

        Emet `blur` sur l'ancien puis `focus` sur le nouveau, dans cet ordre —
        c'est celui de la norme, et une page qui valide un champ au `blur`
        avant d'afficher le suivant en depend.
        """
        if element is self._foyer:
            return False
        ancien, self._foyer = self._foyer, element
        # Un champ editable qui prend le foyer doit avoir un curseur : c'est la
        # que son etat d'edition nait, s'il n'existait pas encore.
        if element is not None:
            self.edition_de(element)
        contexte = self.contexte_js
        if notifie and contexte is not None:
            if ancien is not None:
                contexte.evenement(ancien, "blur", {})
                contexte.evenement(ancien, "focusout", {})
            if element is not None:
                contexte.evenement(element, "focus", {})
                contexte.evenement(element, "focusin", {})
        self._interaction(survoles=self._survole, actifs=None)
        return True

    def elements_focalisables(self):
        """Les elements que `Tab` peut atteindre, dans l'ordre du document.

        L'ordre par `tabindex` positif n'est pas gere : il est rare, souvent
        deconseille, et le simuler a moitie serait pire que de suivre l'ordre du
        document — qui est ce que font les pages bien construites, et donc la
        quasi-totalite des formulaires.
        """
        focalisables = []
        for nœud in self.racine.parcours():
            if not isinstance(nœud, html.Element):
                continue
            if "disabled" in nœud.attributs or "hidden" in nœud.attributs:
                continue
            tabindex = nœud.attributs.get("tabindex")
            if tabindex is not None:
                try:
                    if int(tabindex) < 0:
                        continue
                except ValueError:
                    pass
                focalisables.append(nœud)
                continue
            if nœud.balise in ("input", "select", "textarea", "button"):
                if (nœud.attributs.get("type") or "").lower() == "hidden":
                    continue
                focalisables.append(nœud)
            elif nœud.balise in ("a", "area") and "href" in nœud.attributs:
                focalisables.append(nœud)
        return focalisables

    def deplace_foyer(self, sens=1):
        """`Tab` et `Maj+Tab`. Rend l'element atteint, ou `None`."""
        liste = self.elements_focalisables()
        if not liste:
            return None
        if self._foyer in liste:
            depart = liste.index(self._foyer) + sens
        else:
            depart = 0 if sens > 0 else len(liste) - 1
        # Le foyer fait le tour : c'est ce que fait un navigateur en fin de
        # document, et une page de connexion a trois champs en depend pour
        # revenir au premier.
        cible = liste[depart % len(liste)]
        self.pose_foyer(cible)
        return cible

    def _interaction(self, survoles=None, actifs=None):
        self._survole = survoles
        if not getattr(self.regles, "sensible", False):
            # La feuille ne parle pas d'interaction : rien a recalculer. L'etat
            # est quand meme retenu, pour le jour ou un script insere un
            # `<style>` qui, lui, en parle.
            return False
        etat = (css.lignee(survoles) if survoles is not None else [],
                css.lignee(actifs) if actifs is not None else [],
                css.lignee(self._foyer) if self._foyer is not None else [])
        avant = css.interaction()
        css.pose_interaction(survoles=etat[0], actifs=etat[1], foyer=etat[2])
        if css.interaction() == avant:
            return False
        self.remet_en_page(self.largeur)
        return True

    def element_a(self, x, y):
        """Element sous ce point, en coordonnees de page. `None` sinon.

        Sert a designer la cible d'un clic pour le JavaScript : sans elle, un
        `addEventListener('click', …)` pose sur un bouton ne partirait jamais.
        """
        return _element_a(self.boite, x, y)


def _element_a(boite, x, y):
    if boite is None:
        return None
    if not (boite.x <= x <= boite.x + boite.largeur
            and boite.y <= y <= boite.y + boite.hauteur):
        return None

    style = getattr(boite, "style", None) or {}
    transparente = (style.get("pointer-events", "auto").strip().lower() == "none")

    # Le plus profond gagne : c'est lui que l'utilisateur vise. Et parmi les
    # freres qui se recouvrent, le dernier — celui que la peinture a pose
    # par-dessus. Prendre le premier, comme on le faisait, rendait un calque
    # place plus haut dans le document toujours vainqueur, alors qu'il est
    # dessous a l'ecran.
    #
    # `pointer-events` n'est pas heritee par le test de pointage : un enfant
    # peut remettre `auto` sous un parent a `none`, et c'est meme l'usage le
    # plus courant — un calque qui laisse passer les clics sauf sur son contenu.
    for enfant in reversed(boite.enfants):
        trouve = _element_a(enfant, x, y)
        if trouve is not None:
            return trouve

    # `pointer-events: none` rend la boite transparente au pointeur : le clic
    # traverse et atteint ce qui est dessous. Sans cela, un calque de modale ou
    # une icone decorative avalait chaque clic, et la page devenait inerte sans
    # qu'aucune erreur ne le signale.
    if transparente:
        return None
    return boite.element


def _poids_gras(valeur):
    """Un `font-weight` de `@font-face` est-il gras ?"""
    texte = str(valeur).strip().lower()
    if texte in ("bold", "bolder"):
        return True
    try:
        return int(texte.split()[0]) >= 600
    except (ValueError, IndexError):
        return False


def charge(url, largeur, scripts=True, journal=None):
    """Charge une URL et rend le [`Document`] correspondant."""
    return Document(reseau.charge(url), largeur, scripts=scripts, journal=journal)
