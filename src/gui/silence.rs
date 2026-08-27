//! Ce client annonce-t-il ses trames, ou faut-il deviner ?
//!
//! # Deux drapeaux pour une seule question
//!
//! Un client GUI peut etre de deux sortes. Le moderne dit bonjour, cree sa
//! fenetre et annonce chaque trame : le compositeur ne recopie sa surface que
//! lorsqu'il l'y invite. L'ancien peint dans sa surface sans rien dire : le
//! compositeur ne peut que la recopier periodiquement, « au cas ou ».
//!
//! La distinction se faisait par deux booleens independants, `protocole_actif`
//! et `sans_protocole`, poses a quatre endroits pour le premier et a un seul
//! pour le second. Ils repondent pourtant a la MEME question et ne peuvent pas
//! etre vrais en meme temps.
//!
//! # Le defaut que cela a produit
//!
//! `sans_protocole` est un VERDICT rendu apres un delai de patience, pas une
//! propriete definitive. Ladybird met plus de six secondes a demarrer sous TCG:
//! il depassait le delai, etait declare muet, puis se mettait a parler le
//! protocole. `protocole_actif` passait alors a vrai -- et `sans_protocole`
//! restait vrai, parce que rien ne le levait.
//!
//! Le compositeur recomposait donc a l'aveugle pour toujours. Sur un intervalle
//! mesure au runtime : 94 trames utiles pour 94 recompositions aveugles, les
//! memes. L'inactivite du bureau etait entierement fabriquee par ce verdict
//! perime.
//!
//! # La forme retenue
//!
//! Un seul type, deux transitions nommees, et l'invariant « jamais les deux »
//! tenu par construction plutot que par discipline. Il ne depend de rien --
//! ni horloge, ni tache, ni framebuffer -- et le harnais hote exerce donc le
//! code REEL, pas un modele.

/// Ce que le compositeur sait de la facon dont un client annonce ses trames.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct VerdictProtocole {
    /// Un message valide a prouve que ce client parle le protocole.
    actif: bool,
    /// Le delai de patience a expire sans qu'aucun message n'arrive.
    muet: bool,
}

impl VerdictProtocole {
    pub const fn neuf() -> Self {
        Self { actif: false, muet: false }
    }

    /// Ce client parle le protocole.
    pub const fn protocole_actif(&self) -> bool {
        self.actif
    }

    /// Le compositeur doit-il recopier la surface sans y etre invite ?
    ///
    /// C'est la seule question que l'ordonnancement des trames doit poser. Elle
    /// ne peut pas repondre « oui » a un client qui parle : `actif` gagne
    /// toujours, y compris si un verdict de silence avait ete rendu avant.
    pub const fn recompose_a_l_aveugle(&self) -> bool {
        self.muet && !self.actif
    }

    /// Un message VALIDE prouve que ce client parle le protocole.
    ///
    /// Rend `true` si cela revise un verdict de silence -- l'appelant peut
    /// alors le journaliser. Idempotent : les appels suivants rendent `false`.
    pub fn marque_protocole_actif(&mut self) -> bool {
        let revision = self.muet;
        self.actif = true;
        self.muet = false;
        revision
    }

    /// Le delai de patience a expire.
    ///
    /// Rend `true` si le verdict change reellement. Sans effet sur un client
    /// qui parle deja : un client actif n'est jamais declare muet, meme si
    /// l'appelant le demande, sans quoi un silence passager suffirait a
    /// relancer la recomposition aveugle d'un client parfaitement bavard.
    pub fn declare_muet(&mut self) -> bool {
        if self.actif || self.muet {
            return false;
        }
        self.muet = true;
        true
    }

    /// Un flux invalide retire la preuve : ce client ne parle pas notre
    /// protocole apres tout.
    ///
    /// Ne le declare pas muet pour autant -- c'est au delai de patience de le
    /// faire, et lui seul sait s'il a expire.
    pub fn retire_le_protocole(&mut self) {
        self.actif = false;
    }
}
