//! Un tampon de sortie borne, qui dit quand il a tronque.
//!
//! # Pourquoi pas une `String`
//!
//! Une reponse qui grandit avec ce qu'elle decrit est une allocation sur un
//! chemin qu'on emprunte precisement quand la machine va mal. Le tampon est
//! fixe et vit dans la pile de la connexion.
//!
//! # Pourquoi la troncature se DIT
//!
//! Un tampon qui tronque en silence produit du JSON invalide -- une accolade
//! manquante -- et le client rejette la ligne entiere sans savoir pourquoi.
//! Ici, `tronque()` est vrai, l'appelant le sait, et il envoie une reponse
//! d'erreur courte a la place. Mieux vaut « je n'ai pas pu tout dire » qu'une
//! ligne illisible.

use core::fmt::Write;

pub struct Tampon<const N: usize> {
    octets: [u8; N],
    n: usize,
    tronque: bool,
}

impl<const N: usize> Default for Tampon<N> {
    fn default() -> Self {
        Self::neuf()
    }
}

impl<const N: usize> Tampon<N> {
    pub const fn neuf() -> Self {
        Self { octets: [0; N], n: 0, tronque: false }
    }

    pub fn vide(&mut self) {
        self.n = 0;
        self.tronque = false;
    }

    pub fn tronque(&self) -> bool {
        self.tronque
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn octets(&self) -> &[u8] {
        &self.octets[..self.n]
    }

    /// Revient a une longueur anterieure. Rend `false` si le retour est refuse.
    ///
    /// Pour un producteur qui ecrit un element, constate qu'il ne tient pas, et
    /// prefere le remettre au datagramme suivant plutot que d'en emettre une
    /// moitie. Une ligne JSON coupee est illisible, donc pire qu'absente.
    ///
    /// # Le contrat, et pourquoi il est aussi etroit
    ///
    /// - `longueur > self.n` : REFUSE. Rien ne bouge, et surtout pas le
    ///   drapeau. Une premiere redaction effacait le drapeau dans tous les cas,
    ///   y compris celui-la : un appelant qui se trompait de borne obtenait un
    ///   tampon declare intact alors qu'il avait perdu des octets, ce qui est
    ///   exactement le mensonge que ce type existe pour eviter ;
    /// - `longueur == self.n` : rien n'est defait, donc rien n'est repare. Le
    ///   drapeau est CONSERVE. Sans cette regle, un rollback qui ne rollback
    ///   rien blanchirait une troncature reelle ;
    /// - `longueur < self.n` : le contenu jusqu'a `longueur` est entier par
    ///   construction -- il a ete ecrit avant que la borne soit atteinte -- donc
    ///   le drapeau tombe.
    #[must_use]
    pub fn tronque_a(&mut self, longueur: usize) -> bool {
        if longueur > self.n {
            return false;
        }
        if longueur < self.n {
            self.n = longueur;
            self.tronque = false;
        }
        true
    }

    /// Ajoute le terminateur de ligne du protocole.
    pub fn termine(&mut self) {
        if self.n < N {
            self.octets[self.n] = b'\n';
            self.n += 1;
        } else {
            self.tronque = true;
        }
    }
}

impl<const N: usize> Write for Tampon<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let reste = N - self.n;
        let octets = s.as_bytes();
        if octets.len() > reste {
            self.octets[self.n..].copy_from_slice(&octets[..reste]);
            self.n = N;
            self.tronque = true;
            // On rend `Ok` : l'ecriture continue et l'appelant consultera
            // `tronque()`. Rendre `Err` ferait abandonner `write!` au milieu
            // d'un objet JSON, ce qui ne change rien au probleme et ajoute un
            // chemin d'erreur a chaque appel.
            return Ok(());
        }
        self.octets[self.n..self.n + octets.len()].copy_from_slice(octets);
        self.n += octets.len();
        Ok(())
    }
}
