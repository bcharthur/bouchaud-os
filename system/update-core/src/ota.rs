//! OTA check: is a NEWER, TRUSTED release available? (BOUCHAUD_OTA_V1)
//!
//! The agent downloads two small files from the release source -- the
//! canonical manifest and its Ed25519 signature -- and decides from THEM
//! ONLY. `release.json` (unsigned) is never trusted: everything that matters
//! (version, sequence, hardware family, image digest and size) is in the
//! signed manifest.
//!
//! The decision is the SAME policy `begin` applies (signature by a trusted
//! key, hardware family, anti-rollback floor), checked before a single byte
//! of the image is downloaded; the kernel re-checks it all at `begin`, so a
//! hostile source can at worst waste bandwidth, never get anything installed.
//!
//! Pure function, no I/O: host-tested in tools/update/test_ota.rs.

use crate::anti_retour::{self, Lecture};
use crate::manifeste::{self, CleDeConfiance, Manifeste, RefusManifeste};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The source's release is the running one (or older but at the floor:
    /// a reinstall would be allowed, never offered).
    AJour { sequence: u64 },
    /// Newer, trusted, for this hardware, above the floor: offer it.
    Disponible(Manifeste),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    /// No trusted key configured: fail closed (a production build without a
    /// committed production key never updates).
    AucuneCleDeConfiance,
    SignatureNonFiable,
    ManifesteNonCanonique,
    /// A v1 manifest: no sequence, no anti-rollback -- never offered by OTA.
    SansSequence,
    MaterielIncompatible,
    /// Signed, but below the anti-rollback floor: a downgrade attempt.
    Retrograde,
    RegistreIllisible,
    /// This exact release was installed here and FAILED its health check
    /// (its slot is Bad): offering it again would loop forever
    /// download -> install -> fail -> rollback.
    DejaEchouee,
}

impl Refus {
    pub fn texte(self) -> &'static str {
        match self {
            Refus::AucuneCleDeConfiance => "aucune_cle_de_confiance",
            Refus::SignatureNonFiable => "signature_non_fiable",
            Refus::ManifesteNonCanonique => "manifeste_non_canonique",
            Refus::SansSequence => "manifeste_sans_sequence",
            Refus::MaterielIncompatible => "materiel_incompatible",
            Refus::Retrograde => "version_retrograde",
            Refus::RegistreIllisible => "anti_retour_illisible",
            Refus::DejaEchouee => "release_deja_en_echec_ici",
        }
    }
}

/// `courante`: release sequence of the RUNNING slot (0: unknown, e.g. an
/// image installed before anti-rollback -- then any trusted release above
/// the floor is newer). `echouee`: release sequence that failed its health
/// check on this machine (the other slot is Bad), 0 if none.
pub fn decide(
    texte: &[u8],
    signature: &[u8; 64],
    cles: &[CleDeConfiance],
    lecture: Lecture,
    courante: u64,
    echouee: u64,
    materiel: &str,
) -> Result<Decision, Refus> {
    let (m, _) = manifeste::verifie_signe(texte, signature, cles).map_err(|e| match e {
        RefusManifeste::AucuneCleDeConfiance => Refus::AucuneCleDeConfiance,
        RefusManifeste::SignatureNonFiable => Refus::SignatureNonFiable,
        RefusManifeste::Format => Refus::ManifesteNonCanonique,
    })?;
    if m.sequence == 0 {
        return Err(Refus::SansSequence);
    }
    if m.materiel() != materiel {
        return Err(Refus::MaterielIncompatible);
    }
    let r = anti_retour::autorise(lecture, m.sequence).map_err(|e| match e {
        anti_retour::Refus::Retrograde => Refus::Retrograde,
        anti_retour::Refus::SansSequence => Refus::SansSequence,
        anti_retour::Refus::RegistreIllisible => Refus::RegistreIllisible,
    })?;
    let deja = courante.max(r.plancher);
    if m.sequence <= deja {
        return Ok(Decision::AJour { sequence: m.sequence });
    }
    if echouee != 0 && m.sequence == echouee {
        return Err(Refus::DejaEchouee);
    }
    Ok(Decision::Disponible(m))
}
