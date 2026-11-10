//! P10 — contrat de surface universel, independant des applications.
//!
//! Pour l'instant la politique encadre la presentation du bureau au pilote GPU;
//! la future interface ring3 devra reutiliser les memes validations.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Xrgb8888,
    Argb8888Premultiplie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32, pub y: u32, pub largeur: u32, pub hauteur: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub pid: u32,
    pub identifiant: u32,
    pub generation: u64,
    pub largeur: u32,
    pub hauteur: u32,
    pub pas_octets: u32,
    pub format: Format,
}

pub fn trame_valide(l: usize, h: usize, pixels: usize) -> bool {
    l > 0 && h > 0 && l <= 4096 && h <= 2160
        && l.checked_mul(h) == Some(pixels)
}

impl Surface {
    pub fn valide(&self, demandeur: u32, rect: Rect) -> bool {
        if self.pid != demandeur || self.identifiant == 0
            || self.largeur == 0 || self.hauteur == 0
            || self.largeur > 4096 || self.hauteur > 2160
            || self.pas_octets < self.largeur.saturating_mul(4)
            || self.pas_octets % 4 != 0 || rect.largeur == 0 || rect.hauteur == 0 {
            return false;
        }
        match (rect.x.checked_add(rect.largeur),
               rect.y.checked_add(rect.hauteur)) {
            (Some(x1), Some(y1)) => x1 <= self.largeur && y1 <= self.hauteur,
            _ => false,
        }
    }
}

/// Geometrie des fenetres du WM; le premier compositor GPU ne gere
/// pas encore l'occlusion entre fenetres. Les contacts de bord sont admis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fenetre {
    pub x: i32, pub y: i32, pub w: i32, pub h: i32,
}
pub fn sans_recouvrement(a: Fenetre, b: Fenetre) -> bool {
    if a.w <= 0 || a.h <= 0 || b.w <= 0 || b.h <= 0 { return false; }
    let (ax0, ay0, bx0, by0) = (a.x as i64, a.y as i64, b.x as i64, b.y as i64);
    ax0 + a.w as i64 <= bx0 || bx0 + b.w as i64 <= ax0
        || ay0 + a.h as i64 <= by0 || by0 + b.h as i64 <= ay0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fenetres_gpu_non_recouvrantes() {
        let a = Fenetre { x: 0, y: 0, w: 200, h: 200 };
        assert!(sans_recouvrement(a, Fenetre { x: 200, y: 0, w: 80, h: 100 }));
        assert!(sans_recouvrement(a, Fenetre { x: 0, y: 200, w: 80, h: 100 }));
        assert!(!sans_recouvrement(a, Fenetre { x: 199, y: 100, w: 50, h: 100 }));
        assert!(!sans_recouvrement(a, Fenetre { x: 0, y: 0, w: 0, h: 20 }));
    }
    fn image() -> Surface {
        Surface { pid: 22, identifiant: 17, generation: 1,
                  largeur: 1280, hauteur: 720,
                  pas_octets: 1280 * 4, format: Format::Xrgb8888 }
    }
    #[test]
    fn trame_sans_debordement() {
        assert!(trame_valide(1280, 720, 921600));
        assert!(!trame_valide(1280, 720, 100));
        assert!(!trame_valide(usize::MAX, 2, 0));
    }
    #[test]
    fn isolation_surface_par_pid() {
        let r = Rect { x: 0, y: 0, largeur: 10, hauteur: 10 };
        assert!(image().valide(22, r));
        assert!(!image().valide(23, r));
    }
    #[test]
    fn bornes_et_debordements() {
        let r = Rect { x: 1200, y: 680, largeur: 100, hauteur: 100 };
        assert!(!image().valide(22, r));
        let debord = Rect { x: u32::MAX, y: 0, largeur: 100, hauteur: 1 };
        assert!(!image().valide(22, debord));
    }
    #[test]
    fn pas_trop_petit_refuse() {
        let mut s = image();
        s.pas_octets = 16;
        let r = Rect { x: 0, y: 0, largeur: 1, hauteur: 1 };
        assert!(!s.valide(22, r));
    }
}
