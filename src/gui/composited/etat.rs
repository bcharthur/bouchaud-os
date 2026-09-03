// Le registre des surfaces d'un compositeur, et la propriete des tampons.
//
// CE QUE CE FICHIER EMPECHE
//
// Un compositeur a deux facons de corrompre l'affichage, et les deux sont des
// erreurs de PROPRIETE, pas de dessin :
//
//   1. le client reecrit dans un tampon que le compositeur est en train de
//      lire -- une dechirure, visible comme une bande a mi-hauteur ;
//   2. le compositeur presente un tampon que le client n'a pas fini d'ecrire
//      -- une trame a moitie dessinee.
//
// Aucune des deux ne se corrige par un verrou : le client et le compositeur
// sont deux processus, et un verrou partage entre eux est un verrou qu'un
// client hostile garde. La reponse est la PROPRIETE EXPLICITE : a tout instant,
// chaque tampon appartient a exactement un des deux cotes, et le protocole
// n'accorde le passage que dans un sens a la fois.
//
// LES MESURES
//
// Un compositeur qui « a l'air fluide » ne se debogue pas. Les quatre chiffres
// qui comptent sont ici : trames composees, echeances manquees, intervalle de
// presentation, et pixels sales sur pixels totaux. Le dernier est celui qui
// dit si la composition par region sert a quelque chose : un rapport proche de
// un signifie qu'on recopie l'ecran a chaque trame, et que le degat ne sert a
// rien.

/// A qui appartient un tampon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proprietaire {
    /// Le client peut y ecrire. Le compositeur ne doit pas le lire.
    Client,
    /// Le client a livre sa trame ; le compositeur peut composer.
    Compositeur,
    /// Le compositeur l'a presente ; il reste a l'ecran jusqu'a la trame
    /// suivante, et personne ne doit y ecrire.
    Affiche,
}

/// Une surface suivie par le compositeur.
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub id: u32,
    /// Le processus proprietaire. Une trame livree par un autre est refusee :
    /// c'est la seule chose qui empeche un client de piloter la surface d'un
    /// autre.
    pub client: u32,
    pub largeur: u32,
    pub hauteur: u32,
    pub pas: u32,
    pub echelle: u32,
    pub proprietaires: [Proprietaire; TAMPONS],
    /// Degat accumule depuis la derniere composition.
    pub degat: Rect,
    /// Derniere trame livree par le client.
    pub derniere_trame: u32,
    /// Tampon actuellement affiche, s'il y en a un.
    pub affiche: Option<u32>,
    pub vivante: bool,
}

impl Surface {
    fn neuve(id: u32, client: u32, largeur: u32, hauteur: u32, echelle: u32) -> Self {
        let mut proprietaires = [Proprietaire::Client; TAMPONS];
        // Un seul tampon appartient au client au depart. L'autre est libre --
        // represente comme « au compositeur », qui le rendra a la premiere
        // presentation. Donner les deux d'emblee laisserait le client livrer
        // deux trames avant toute composition, et la premiere serait perdue
        // sans que rien ne le dise.
        for tampon in 1..TAMPONS {
            proprietaires[tampon] = Proprietaire::Compositeur;
        }
        Self {
            id,
            client,
            largeur,
            hauteur,
            pas: largeur.saturating_mul(4),
            echelle,
            proprietaires,
            degat: Rect::default(),
            derniere_trame: 0,
            affiche: None,
            vivante: true,
        }
    }

    /// Octets d'un tampon.
    pub const fn octets_tampon(&self) -> u64 {
        self.pas as u64 * self.hauteur as u64
    }

    /// Octets de la region partagee entiere.
    pub const fn octets_region(&self) -> u64 {
        self.octets_tampon() * TAMPONS as u64
    }
}

/// Mesures d'un compositeur. Ce qui distingue « ca a l'air fluide » d'une
/// affirmation verifiable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mesures {
    pub trames_composees: u64,
    pub trames_presentees: u64,
    /// Trames livrees et jamais composees : le client a livre deux fois avant
    /// une presentation.
    pub trames_ecrasees: u64,
    /// Compositions terminees apres l'echeance de la trame.
    pub echeances_manquees: u64,
    /// Intervalle de presentation le plus long observe.
    pub intervalle_max_ns: u64,
    /// Somme des pixels sales composes.
    pub pixels_sales: u64,
    /// Somme des pixels qu'une composition PLEINE aurait recopies.
    pub pixels_total: u64,
    /// Messages refuses, par famille.
    pub refus_propriete: u64,
    pub refus_client: u64,
    pub refus_geometrie: u64,
}

impl Mesures {
    /// Part des pixels reellement recopies, en millemes.
    ///
    /// Proche de mille : la composition par region ne sert a rien, on recopie
    /// l'ecran a chaque trame. C'est le chiffre qui dit s'il faut travailler
    /// le degat ou le presentateur.
    pub fn taux_degat_millemes(&self) -> u64 {
        if self.pixels_total == 0 { return 0; }
        (self.pixels_sales.saturating_mul(1000)) / self.pixels_total
    }
}

/// Le registre des surfaces. Tableau de taille fixe : un compositeur qui alloue
/// par surface se fait epuiser par un client qui en demande sans fin.
pub struct Registre {
    surfaces: [Option<Surface>; SURFACES_MAX],
    prochaine_id: u32,
    echelle: u32,
    pub mesures: Mesures,
    /// Instant de la derniere presentation, pour l'intervalle.
    derniere_presentation_ns: u64,
}

impl Default for Registre {
    fn default() -> Self { Self::neuf(ECHELLE_PAR_DEFAUT) }
}

const ECHELLE_PAR_DEFAUT: u32 = ECHELLE_UNITE;

impl Registre {
    pub const fn neuf(echelle: u32) -> Self {
        Self {
            surfaces: [None; SURFACES_MAX],
            prochaine_id: 1,
            echelle,
            mesures: Mesures {
                trames_composees: 0,
                trames_presentees: 0,
                trames_ecrasees: 0,
                echeances_manquees: 0,
                intervalle_max_ns: 0,
                pixels_sales: 0,
                pixels_total: 0,
                refus_propriete: 0,
                refus_client: 0,
                refus_geometrie: 0,
            },
            derniere_presentation_ns: 0,
        }
    }

    pub fn echelle(&self) -> u32 { self.echelle }

    /// Accorde une surface a un client.
    ///
    /// La geometrie demandee est LOGIQUE ; la surface rendue est PHYSIQUE.
    pub fn accorde(
        &mut self,
        client: u32,
        largeur_logique: u32,
        hauteur_logique: u32,
    ) -> Result<Surface, Refus> {
        if largeur_logique == 0 || hauteur_logique == 0
            || largeur_logique > LARGEUR_MAX || hauteur_logique > HAUTEUR_MAX
        {
            self.mesures.refus_geometrie += 1;
            return Err(Refus::GeometrieInvalide);
        }
        if self.surfaces.iter().flatten().any(|s| s.vivante && s.client == client) {
            self.mesures.refus_client += 1;
            return Err(Refus::DejaAttache);
        }
        let Some(place) = self.surfaces.iter().position(|s| s.is_none()) else {
            return Err(Refus::PlusDeSurface);
        };
        let largeur = longueur_physique(largeur_logique, self.echelle);
        let hauteur = longueur_physique(hauteur_logique, self.echelle);
        let surface = Surface::neuve(self.prochaine_id, client, largeur, hauteur, self.echelle);
        self.prochaine_id = self.prochaine_id.wrapping_add(1).max(1);
        self.surfaces[place] = Some(surface);
        Ok(surface)
    }

    fn index(&self, id: u32) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|s| matches!(s, Some(s) if s.id == id && s.vivante))
    }

    pub fn surface(&self, id: u32) -> Option<&Surface> {
        self.surfaces[self.index(id)?].as_ref()
    }

    pub fn vivantes(&self) -> usize {
        self.surfaces.iter().flatten().filter(|s| s.vivante).count()
    }

    /// Une trame livree par un client.
    ///
    /// Trois refus, et chacun protege un cas different :
    ///
    ///   * surface inconnue -- un client qui parle d'une surface detachee ;
    ///   * mauvais client -- un client qui pilote la surface d'un autre ;
    ///   * tampon non possede -- LE cas qui corrompt l'affichage. Un client qui
    ///     livre un tampon appartenant au compositeur demande a ce dernier de
    ///     lire pendant qu'il ecrit.
    pub fn trame_livree(
        &mut self,
        client: u32,
        trame: &TrameLivree,
    ) -> Result<(), Refus> {
        let Some(index) = self.index(trame.surface) else {
            self.mesures.refus_client += 1;
            return Err(Refus::Inconnue);
        };
        let echelle = self.echelle;
        let surface = self.surfaces[index].as_mut().expect("index verifie");
        if surface.client != client {
            self.mesures.refus_client += 1;
            return Err(Refus::Inconnue);
        }
        let tampon = trame.tampon as usize;
        if tampon >= TAMPONS || surface.proprietaires[tampon] != Proprietaire::Client {
            self.mesures.refus_propriete += 1;
            return Err(Refus::TamponNonPossede);
        }

        // Le degat est ROGNE avant d'etre accumule. Un client peut se tromper
        // ou mentir ; les deux se traitent pareil, et sans ce rognage un
        // `degat.x = -1` ferait lire le compositeur avant le debut du tampon.
        let _ = echelle;
        let rogne = rogne_degat(trame.degat, surface.largeur, surface.hauteur);
        if surface.proprietaires[tampon] == Proprietaire::Client
            && surface.degat_en_attente()
        {
            // Le client livre une seconde trame avant toute composition : la
            // premiere ne sera jamais vue. Ce n'est pas une faute -- c'est ce
            // que fait un client plus rapide que l'ecran -- mais cela se
            // COMPTE, sinon un rendu qui produit deux fois trop de trames
            // ressemble a un compositeur lent.
            self.mesures.trames_ecrasees += 1;
        }
        surface.degat = surface.degat.union(&rogne);
        surface.derniere_trame = trame.trame;
        surface.proprietaires[tampon] = Proprietaire::Compositeur;
        Ok(())
    }

    /// Compose toutes les surfaces qui ont du travail, et rend les tampons
    /// liberes.
    ///
    /// Rend la liste des `(surface, tampon, trame)` a annoncer aux clients :
    /// c'est le pas 7 du tranchant vertical, celui qu'on oublie et qui fait
    /// dechirer l'affichage.
    pub fn compose(&mut self, maintenant_ns: u64, echeance_ns: u64) -> Vec<(u32, u32, u32)> {
        let mut rendus = Vec::new();
        let mut a_presente = false;

        for place in 0..SURFACES_MAX {
            let Some(surface) = self.surfaces[place].as_mut() else { continue };
            if !surface.vivante { continue; }
            let Some(pret) = surface
                .proprietaires
                .iter()
                .position(|p| *p == Proprietaire::Compositeur)
            else { continue };
            if surface.degat.vide() { continue; }

            let pixels = surface.degat.largeur as u64 * surface.degat.hauteur as u64;
            self.mesures.pixels_sales += pixels;
            self.mesures.pixels_total += surface.largeur as u64 * surface.hauteur as u64;
            self.mesures.trames_composees += 1;
            surface.degat = Rect::default();

            // Le tampon precedemment affiche redevient celui du client. C'est
            // ici, et seulement ici, que le client retrouve le droit d'ecrire.
            if let Some(ancien) = surface.affiche {
                if (ancien as usize) < TAMPONS && ancien as usize != pret {
                    surface.proprietaires[ancien as usize] = Proprietaire::Client;
                    rendus.push((surface.id, ancien, surface.derniere_trame));
                }
            } else {
                // Premiere trame : le tampon libre revient au client.
                for autre in 0..TAMPONS {
                    if autre != pret && surface.proprietaires[autre] == Proprietaire::Compositeur {
                        surface.proprietaires[autre] = Proprietaire::Client;
                        rendus.push((surface.id, autre as u32, surface.derniere_trame));
                    }
                }
            }
            surface.proprietaires[pret] = Proprietaire::Affiche;
            surface.affiche = Some(pret as u32);
            a_presente = true;
        }

        if a_presente {
            self.mesures.trames_presentees += 1;
            if self.derniere_presentation_ns != 0 {
                let intervalle = maintenant_ns.saturating_sub(self.derniere_presentation_ns);
                if intervalle > self.mesures.intervalle_max_ns {
                    self.mesures.intervalle_max_ns = intervalle;
                }
            }
            self.derniere_presentation_ns = maintenant_ns;
            if echeance_ns != 0 && maintenant_ns > echeance_ns {
                self.mesures.echeances_manquees += 1;
            }
        }
        rendus
    }

    /// Le client abandonne sa surface.
    pub fn detache(&mut self, client: u32, id: u32) -> Result<(), Refus> {
        let Some(index) = self.index(id) else { return Err(Refus::Inconnue) };
        let surface = self.surfaces[index].as_mut().expect("index verifie");
        if surface.client != client {
            self.mesures.refus_client += 1;
            return Err(Refus::Inconnue);
        }
        self.surfaces[index] = None;
        Ok(())
    }

    /// Le client est mort. Sa surface disparait sans qu'il ait a le demander.
    ///
    /// Sans cela, un moteur de rendu qui tombe laisserait sa surface occuper un
    /// emplacement pour toujours -- et trente-deux plantages epuiseraient le
    /// registre.
    pub fn oublie_client(&mut self, client: u32) -> usize {
        let mut retirees = 0;
        for place in 0..SURFACES_MAX {
            if matches!(&self.surfaces[place], Some(s) if s.client == client) {
                self.surfaces[place] = None;
                retirees += 1;
            }
        }
        retirees
    }
}

impl Surface {
    fn degat_en_attente(&self) -> bool {
        !self.degat.vide()
    }
}

/// Bornes de geometrie. Un client qui demande huit mille par huit mille
/// obtiendrait une region de 256 Mio par tampon.
pub const LARGEUR_MAX: u32 = 8192;
pub const HAUTEUR_MAX: u32 = 8192;
