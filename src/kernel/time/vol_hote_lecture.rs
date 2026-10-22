//! Lecture d'un compteur que l'hyperviseur publie sous version
//! (BOUCHAUD_PREUVE_HOTE_V1). Pure : testee sur l'hote
//! (`tools/smp/test_vol_hote.rs`).

/// Lecture d'un compteur publie sous version (pure ; testee sur l'hote).
///
/// `lis()` rend `(version, steal)` relus dans cet ordre, puis `version()` la
/// version relue apres. Une version impaire (ecriture en cours) ou qui a
/// change entre les deux lectures fait recommencer, au plus `essais` fois :
/// `None` si l'hote n'a jamais laisse une lecture stable.
pub fn lecture_versionnee(
    mut lis: impl FnMut() -> (u32, u64),
    mut version: impl FnMut() -> u32,
    essais: u32,
) -> Option<u64> {
    for _ in 0..essais {
        let (v1, vol) = lis();
        if v1 & 1 == 1 {
            continue;
        }
        if version() == v1 {
            return Some(vol);
        }
    }
    None
}
