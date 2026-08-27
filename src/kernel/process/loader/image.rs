//! Ce qu'un chargeur produit, et ce qu'un espace d'adressage consomme.
//!
//! # Pourquoi une representation intermediaire
//!
//! ELF et PE decrivent la meme chose de deux facons : des octets de fichier a
//! poser a des adresses, avec des droits. Tout le reste -- en-tetes, tables,
//! relocations, conventions de nommage -- est du detail de format.
//!
//! Sans cette frontiere, chaque format doit connaitre l'espace d'adressage, et
//! chaque evolution de l'espace d'adressage doit etre repercutee dans chaque
//! format. C'est exactement l'enchevetrement dont `exec` sortait.
//!
//! [`ImagePreparee`] est donc volontairement pauvre : une base, un point
//! d'entree, une liste de segments. Elle ne sait pas ce qu'est une section, un
//! import ou une relocation.
//!
//! # Pourquoi elle ne porte AUCUN octet
//!
//! On pourrait fabriquer ici le tampon final -- `SizeOfImage` octets, sections
//! recopiees, BSS a zero -- et le rendre pret a projeter. Ce serait plus simple
//! a mapper, et cela couterait une allocation de la taille de l'image, plus une
//! copie complete, pour chaque `exec`.
//!
//! Les segments designent donc leur source dans le fichier d'origine
//! (`offset_source`, `taille_source`) et laissent le projeteur copier
//! directement du fichier vers l'espace cible. Ce qui depasse `taille_source`
//! est du zero -- BSS et bourrage d'alignement --, et n'a aucune source a lire.
//!
//! # Preparer n'est pas projeter
//!
//! Rien ici ne touche a un espace d'adressage, a une table des taches ou au
//! gros verrou. C'est une transformation de `&[u8]` en description, donc du
//! travail purement local : c'est precisement la part qui pourra sortir du gros
//! verrou quand `execve` sera scinde en `prepare` / `commit`.

/// Nombre maximum de segments d'une image.
///
/// Fixe, comme `lit_sections` : pas d'allocateur sur ce chemin, et une image
/// qui en demanderait davantage est refusee plutot que tronquee en silence.
/// Une section PE donne un segment, plus un pour les en-tetes.
pub const MAX_SEGMENTS: usize = 33;

/// Droits d'un segment, independants du format d'origine.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Droits {
    pub lecture: bool,
    pub ecriture: bool,
    pub execution: bool,
}

impl Droits {
    pub const fn lecture_seule() -> Self {
        Self { lecture: true, ecriture: false, execution: false }
    }

    /// Une image qui demande a la fois l'ecriture et l'execution est refusee
    /// par les chargeurs bien avant d'arriver ici. Le predicat reste pour que
    /// le projeteur puisse le REVERIFIER : c'est la derniere barriere avant que
    /// des pages reellement inscriptibles-et-executables n'existent.
    pub const fn viole_w_xor_x(&self) -> bool {
        self.ecriture && self.execution
    }
}

/// Un intervalle d'adresses a projeter, et d'ou en viennent les octets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Segment {
    /// Adresse virtuelle finale, base de chargement comprise.
    pub adresse: u64,
    /// Taille en memoire. Toujours `>= taille_source` ; le reste est du zero.
    pub taille: usize,
    /// Offset des octets dans le fichier d'origine.
    pub offset_source: usize,
    /// Nombre d'octets a recopier depuis le fichier. Zero pour un segment
    /// entierement fabrique (BSS).
    pub taille_source: usize,
    pub droits: Droits,
}

impl Segment {
    /// Octets a mettre a zero apres la copie.
    pub const fn taille_zero(&self) -> usize {
        self.taille.saturating_sub(self.taille_source)
    }

    /// Fin exclusive du segment, `None` si le calcul deborde.
    pub fn fin(&self) -> Option<u64> {
        self.adresse.checked_add(self.taille as u64)
    }
}

/// Une image prete a etre projetee dans un espace d'adressage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImagePreparee {
    /// Base de chargement retenue.
    pub base: u64,
    /// Point d'entree ABSOLU -- base comprise, pas une RVA.
    pub point_entree: u64,
    /// Etendue totale reservee, depuis `base`.
    pub taille_image: usize,
    /// Ecart entre la base retenue et celle inscrite dans le fichier. Nul quand
    /// l'image est chargee la ou elle le demande ; c'est ce que les relocations
    /// doivent ajouter sinon.
    pub decalage: i64,
    segments: [Segment; MAX_SEGMENTS],
    nombre: usize,
}

/// Pourquoi une image preparee ne tient pas debout.
///
/// Distinct des refus propres a chaque format : ceux-ci portent sur la
/// DESCRIPTION produite, pas sur les octets d'entree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefusImage {
    /// Plus de segments que la capacite fixe.
    TropDeSegments { demandes: usize, capacite: usize },
    /// Un segment sort de `[base, base + taille_image)`.
    SegmentHorsImage { adresse: u64 },
    /// Un segment lit au-dela de la fin du fichier.
    SourceHorsFichier { offset: usize, taille: usize },
    /// Deux segments se recouvrent : le resultat dependrait de l'ordre de
    /// projection, donc du chargeur et non du binaire.
    SegmentsQuiSeRecouvrent { premier: u64, second: u64 },
    /// Un segment demande l'ecriture ET l'execution.
    EcritureEtExecution { adresse: u64 },
    /// Le point d'entree ne tombe dans aucun segment executable.
    PointEntreeInvalide { adresse: u64 },
    /// Aucun segment : il n'y aurait rien a executer.
    Vide,
}

impl ImagePreparee {
    pub const fn neuve(base: u64, point_entree: u64, taille_image: usize, decalage: i64) -> Self {
        Self {
            base,
            point_entree,
            taille_image,
            decalage,
            segments: [Segment {
                adresse: 0,
                taille: 0,
                offset_source: 0,
                taille_source: 0,
                droits: Droits { lecture: false, ecriture: false, execution: false },
            }; MAX_SEGMENTS],
            nombre: 0,
        }
    }

    pub fn ajoute(&mut self, segment: Segment) -> Result<(), RefusImage> {
        if self.nombre == MAX_SEGMENTS {
            return Err(RefusImage::TropDeSegments {
                demandes: self.nombre + 1,
                capacite: MAX_SEGMENTS,
            });
        }
        self.segments[self.nombre] = segment;
        self.nombre += 1;
        Ok(())
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments[..self.nombre]
    }

    pub fn est_vide(&self) -> bool {
        self.nombre == 0
    }

    /// Derniere barriere avant la projection.
    ///
    /// Chaque verification a deja son equivalent cote format -- et c'est
    /// voulu : celles-ci portent sur la description PRODUITE, pas sur les
    /// octets d'entree. Un chargeur qui traduit mal une image pourtant valide
    /// est aussi dangereux qu'une image invalide, et rien d'autre ne l'aurait
    /// vu.
    pub fn valide(&self, taille_fichier: usize) -> Result<(), RefusImage> {
        if self.est_vide() {
            return Err(RefusImage::Vide);
        }
        let fin_image = self
            .base
            .checked_add(self.taille_image as u64)
            .ok_or(RefusImage::SegmentHorsImage { adresse: self.base })?;

        for segment in self.segments() {
            if segment.droits.viole_w_xor_x() {
                return Err(RefusImage::EcritureEtExecution { adresse: segment.adresse });
            }
            let fin = segment
                .fin()
                .ok_or(RefusImage::SegmentHorsImage { adresse: segment.adresse })?;
            if segment.adresse < self.base || fin > fin_image {
                return Err(RefusImage::SegmentHorsImage { adresse: segment.adresse });
            }
            if segment.taille_source > segment.taille {
                return Err(RefusImage::SourceHorsFichier {
                    offset: segment.offset_source,
                    taille: segment.taille_source,
                });
            }
            let fin_source = segment
                .offset_source
                .checked_add(segment.taille_source)
                .ok_or(RefusImage::SourceHorsFichier {
                    offset: segment.offset_source,
                    taille: segment.taille_source,
                })?;
            if segment.taille_source != 0 && fin_source > taille_fichier {
                return Err(RefusImage::SourceHorsFichier {
                    offset: segment.offset_source,
                    taille: segment.taille_source,
                });
            }
        }

        // Recouvrements : quadratique, sur au plus 33 segments.
        let liste = self.segments();
        for (index, segment) in liste.iter().enumerate() {
            if segment.taille == 0 {
                continue;
            }
            let fin = segment.adresse + segment.taille as u64;
            for autre in &liste[index + 1..] {
                if autre.taille == 0 {
                    continue;
                }
                let autre_fin = autre.adresse + autre.taille as u64;
                if segment.adresse < autre_fin && autre.adresse < fin {
                    return Err(RefusImage::SegmentsQuiSeRecouvrent {
                        premier: segment.adresse,
                        second: autre.adresse,
                    });
                }
            }
        }

        let entree_executable = liste.iter().any(|segment| {
            segment.droits.execution
                && self.point_entree >= segment.adresse
                && self.point_entree < segment.adresse + segment.taille as u64
        });
        if !entree_executable {
            return Err(RefusImage::PointEntreeInvalide { adresse: self.point_entree });
        }

        Ok(())
    }
}
