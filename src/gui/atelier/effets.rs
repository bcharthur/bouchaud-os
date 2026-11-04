//! Effets Atelier : ouverture et fermeture de fenetre composees
//! (BOUCHAUD_ATELIER_EFFETS_V1).
//!
//! # Comment une fenetre s'ouvre sans surface independante
//!
//! Le compositeur peint chaque calque DIRECTEMENT dans le tampon arriere, sous
//! une decoupe. Une fenetre n'a donc pas d'image a elle qu'on pourrait fondre.
//! Plutot que de faire semblant, on en fabrique une, le temps d'une decoupe :
//!
//!   1. on garde ce qui est sous la fenetre (les calques inferieurs, deja
//!      peints dans cette decoupe) ;
//!   2. on peint la fenetre, normalement ;
//!   3. on recopie ce qu'elle a peint, et on remet le dessous ;
//!   4. on melange la copie, DECALEE de `dy` et a l'opacite de l'instant, sur
//!      le dessous.
//!
//! Les pixels touches restent dans la zone que la trame recompose (l'union de
//! la position finale et de la position decalee), et tout ce qui est hors de
//! la decoupe est remis a l'identique avant la presentation : rien de ce qui
//! est presente n'est un etat intermediaire.
//!
//! # Comment une fenetre se ferme sans surface liberee
//!
//! A la fermeture, AVANT que la fenetre ne quitte la liste (et avant que son
//! client ne soit termine), elle est peinte une derniere fois dans le tampon,
//! copiee dans un INSTANTANE qui nous appartient, puis le tampon est remis a
//! l'identique. L'animation ne lit plus que cette copie : aucune surface,
//! aucun client, rien qui puisse etre libere sous elle. L'instantane est
//! libere quand l'animation finit, quand elle est evincee, ou a la fermeture
//! du bureau.
//!
//! # Ce qui ne s'anime pas
//!
//! * en mode `Aucun` : rien n'est lance, la boucle du bureau est celle d'avant ;
//! * en composition directe dans la memoire du scanout virtio (`gpu-tampon
//!   direct`) : peindre hors trame y serait un acces pendant transfert. La
//!   fermeture y est instantanee ;
//! * un client mort : sa disparition n'est pas un geste de l'utilisateur.

use alloc::vec::Vec;
use super::animations::{self as an, Animation, Nature, Registre};
use super::dock::Boite;
use crate::gui::framebuffer as fb;

/// Plus grand instantane accepte, en pixels (une fenetre plein ecran 1920x1080).
const INSTANTANE_MAX_PIXELS: usize = 1920 * 1080;
/// Cle des instantanes : bit de poids fort, jamais un `WindowId`.
const CLE_INSTANTANE: u64 = 1 << 63;

struct Instantane {
    cle: u64,
    cadre: Boite,
    rayon: u32,
    pixels: Vec<u32>,
}

static mut REGISTRE: Registre = Registre::neuf();
static mut INSTANTANES: Vec<Instantane> = Vec::new();
static mut A_RECOMPOSER: Vec<Boite> = Vec::new();
static mut DESSOUS: Vec<u32> = Vec::new();
static mut DESSUS: Vec<u32> = Vec::new();
static mut PROCHAINE_CLE: u64 = CLE_INSTANTANE;
static mut HORLOGE_TRAME: u64 = 0;
static mut NS_TRAME: u64 = 0;
static mut ANIME_CETTE_TRAME: bool = false;
static mut TRAMES_ANIMEES: u32 = 0;
static mut NS_ANIMES: u64 = 0;
static mut NS_PIRE: u64 = 0;
/// Le bureau a-t-il deja pose sa premiere trame ? Avant elle, rien ne
/// s'anime : les fenetres ouvertes au demarrage apparaissent avec le bureau,
/// et animer la plus lourde des trames ne ferait que la retarder.
static mut BUREAU_POSE: bool = false;
/// Octets retenus par les instantanes, et leur pic depuis le dernier releve.
static mut INSTANTANES_OCTETS: usize = 0;
static mut INSTANTANES_PIC: usize = 0;
/// Une fenetre vient de s'ouvrir : la prochaine trame mesure sa latence,
/// animation ou pas (mode `Aucun` compris), pour comparer les modes.
static mut MESURE_OUVERTURE: bool = false;

// LE FIL UNIQUE, VERIFIE ET NON SUPPOSE.
//
// Tout l'etat de ce module est `static mut` : il n'est sain que si un seul fil
// y touche. C'est le fil du bureau (`window_manager::boucle`), qui s'en declare
// proprietaire au demarrage. Un appel depuis un autre fil est REFUSE (et dit
// une fois) au lieu de courir le risque d'une course : l'animation cede, la
// memoire reste coherente.
static PROPRIETAIRE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
static ETRANGER_SIGNALE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn tid_courant() -> Option<u32> {
    crate::kernel::task::try_current().map(|t| t.tid)
}

/// Le fil du bureau se declare proprietaire de l'etat des effets.
pub fn adopte_fil() {
    if let Some(tid) = tid_courant() {
        PROPRIETAIRE.store(tid, core::sync::atomic::Ordering::Release);
    }
}

/// Une trame du fil proprietaire est-elle en cours de composition ?
///
/// Les LECTEURS (plan de scene, peintres) ne sont appeles que pendant la
/// composition, entre `debut_trame` et `fin_trame`, que seul le proprietaire
/// verifie peut ouvrir. Ils testent ce drapeau plutot que d'interroger le
/// registre des taches a chaque calque : cet appel, des dizaines de fois par
/// trame, coutait ~400 ms a la premiere trame sous TCG.
static EN_TRAME: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn lecture_ok() -> bool {
    use core::sync::atomic::Ordering;
    PROPRIETAIRE.load(Ordering::Acquire) == u32::MAX || EN_TRAME.load(Ordering::Acquire)
}

/// L'appelant est-il le fil proprietaire ? Avant toute adoption (apercu de
/// demarrage), seul le registre vide est visible : rien a proteger.
fn fil_ok() -> bool {
    use core::sync::atomic::Ordering;
    let p = PROPRIETAIRE.load(Ordering::Acquire);
    if p == u32::MAX {
        return true;
    }
    if tid_courant() == Some(p) {
        return true;
    }
    if !ETRANGER_SIGNALE.swap(true, Ordering::Relaxed) {
        crate::serial_println!("BOUCHAUD_ATELIER_EFFETS_FIL_ETRANGER proprietaire={} appelant={:?}",
            p, tid_courant());
    }
    false
}

// SAFETY (tout le module) : fil unique du gestionnaire de fenetres.
fn registre() -> &'static mut Registre {
    unsafe { &mut *core::ptr::addr_of_mut!(REGISTRE) }
}
fn instantanes() -> &'static mut Vec<Instantane> {
    unsafe { &mut *core::ptr::addr_of_mut!(INSTANTANES) }
}
fn a_recomposer() -> &'static mut Vec<Boite> {
    unsafe { &mut *core::ptr::addr_of_mut!(A_RECOMPOSER) }
}

fn mode() -> an::Mode {
    super::reglages::animations()
}

fn composition_directe() -> bool {
    crate::drivers::gfx::compteurs_tampon().1
}

/// Une animation vient de sortir du registre (finie, evincee ou annulee) :
/// sa derniere zone sera recomposee, son instantane libere.
fn retire(a: Animation) {
    a_recomposer().push(a.bornes);
    if a.nature == Nature::Fermeture {
        let liberes: usize = instantanes().iter().filter(|i| i.cle == a.cle)
            .map(|i| i.pixels.capacity() * 4).sum();
        instantanes().retain(|i| i.cle != a.cle);
        unsafe { INSTANTANES_OCTETS = INSTANTANES_OCTETS.saturating_sub(liberes); }
    }
}

fn decale(b: Boite, dy: i32) -> Boite {
    Boite { y: b.y + dy, ..b }
}

/// Zone d'une ouverture a la progression courante : position finale et
/// position decalee.
fn zone_ouverture(peinte: Boite, dy: i32) -> Boite {
    an::union(peinte, decale(peinte, dy))
}

fn agrandit(b: Boite, n: i32) -> Boite {
    Boite { x: b.x - n, y: b.y - n, w: b.w + 2 * n, h: b.h + 2 * n }
}

/// Une fenetre apparait. `peinte` : ses bornes peintes (ombre comprise).
pub fn ouvre(cle: u64, peinte: Boite) {
    if !fil_ok() {
        return;
    }
    unsafe { MESURE_OUVERTURE = BUREAU_POSE; }
    let duree = an::duree_ms(mode(), Nature::Ouverture);
    if duree == 0 || !unsafe { BUREAU_POSE } {
        return;
    }
    let zone = zone_ouverture(peinte, an::decalage(mode(), 0));
    if let Some(e) = registre().lance(Animation {
        cle, nature: Nature::Ouverture, debut_ms: an::EN_ATTENTE, duree_ms: duree, depart: 0,
        bornes: zone,
    }) {
        retire(e);
    }
}

/// Une fenetre disparait. `cadre` : son rectangle exterieur ; `dessine` la
/// peint telle qu'elle est (appele une fois, ici, avant toute liberation).
pub fn ferme(cle_fenetre: u64, cadre: Boite, rayon: u32, dessine: impl FnOnce()) {
    if !fil_ok() {
        return;
    }
    // Une ouverture en cours s'arrete ici ; la fermeture part de son opacite.
    let mut depart = 255u8;
    if let Some(o) = registre().annule(cle_fenetre, Nature::Ouverture) {
        let p = o.progression(crate::kernel::timer::monotonic_ms());
        depart = an::opacite(Nature::Ouverture, 0, p).max(1);
        retire(o);
    }
    let duree = an::duree_ms(mode(), Nature::Fermeture);
    if duree == 0 || composition_directe() || !unsafe { BUREAU_POSE } {
        return;
    }
    let (l, h) = (fb::width() as i32, fb::height() as i32);
    let x0 = cadre.x.max(0);
    let y0 = cadre.y.max(0);
    let x1 = (cadre.x + cadre.w).min(l);
    let y1 = (cadre.y + cadre.h).min(h);
    if x1 <= x0 || y1 <= y0 || ((x1 - x0) * (y1 - y0)) as usize > INSTANTANE_MAX_PIXELS {
        return;
    }
    let visible = Boite { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
    let n = (visible.w * visible.h) as usize;
    if !an::admet_instantane(unsafe { INSTANTANES_OCTETS }, n * 4) {
        crate::serial_println!("BOUCHAUD_ATELIER_INSTANTANE refuse octets={} retenus={} budget={}",
            n * 4, unsafe { INSTANTANES_OCTETS }, an::BUDGET_INSTANTANES_OCTETS);
        return;
    }
    let mut pixels = Vec::new();
    if pixels.try_reserve_exact(n).is_err() {
        return;
    }
    // Peindre, copier, remettre : le tampon ressort identique. Le dessous
    // passe par le tampon de travail partage, pas par une seconde allocation.
    let clip = fb::clip_rect();
    let dessous = unsafe { &mut *core::ptr::addr_of_mut!(DESSOUS) };
    if dessous.try_reserve(n.saturating_sub(dessous.capacity())).is_err() {
        return;
    }
    copie(visible, dessous);
    fb::set_clip(x0 as usize, y0 as usize, visible.w as usize, visible.h as usize);
    dessine();
    copie(visible, &mut pixels);
    pose(visible, dessous);
    fb::set_clip(clip.0, clip.1, clip.2.saturating_sub(clip.0), clip.3.saturating_sub(clip.1));

    let cle = unsafe {
        let c = PROCHAINE_CLE;
        PROCHAINE_CLE = PROCHAINE_CLE.wrapping_add(1) | CLE_INSTANTANE;
        c
    };
    unsafe {
        INSTANTANES_OCTETS += pixels.capacity() * 4;
        INSTANTANES_PIC = INSTANTANES_PIC.max(INSTANTANES_OCTETS);
    }
    instantanes().push(Instantane { cle, cadre: visible, rayon, pixels });
    let bornes = agrandit(visible, crate::gui::disposition::DEBORD_OMBRE as i32);
    if let Some(e) = registre().lance(Animation {
        cle, nature: Nature::Fermeture, debut_ms: an::EN_ATTENTE,
        duree_ms: duree, depart, bornes,
    }) {
        retire(e);
    }
}

/// Appele a chaque tour du bureau, avant la composition. Verse dans `ajoute`
/// chaque zone a recomposer, et rend `true` s'il faut une trame.
///
/// `peinte(cle)` rend les bornes peintes ACTUELLES de la fenetre `cle`, ou
/// `None` si elle n'est plus visible (fermee, reduite) : son ouverture
/// s'arrete alors, et sa derniere zone est recomposee.
pub fn tour(maintenant: u64, peinte: impl Fn(u64) -> Option<Boite>, mut ajoute_zone: impl FnMut(Boite)) -> bool {
    if !fil_ok() {
        return false;
    }
    let mut finies = 0u32;
    let mut zones = 0u32;
    let mut ajoute = |b: Boite| {
        zones += 1;
        ajoute_zone(b);
    };
    registre().termine(maintenant, |a| {
        retire(a);
        finies += 1;
    });
    // Les ouvertures suivent leur fenetre, qui peut bouger pendant l'animation.
    let mut perdues: [u64; an::CAPACITE] = [0; an::CAPACITE];
    let mut n = 0;
    let cles: Vec<(u64, Nature)> = registre().en_cours().map(|a| (a.cle, a.nature)).collect();
    for (cle, nature) in cles {
        match nature {
            Nature::Ouverture => match peinte(cle) {
                Some(p) => {
                    let a = registre().cherche(cle, nature).unwrap();
                    let dy = an::decalage(mode(), a.progression(maintenant));
                    if let Some(z) = registre().recouvre(cle, nature, zone_ouverture(p, dy)) {
                        ajoute(z);
                    }
                }
                None => {
                    perdues[n] = cle;
                    n += 1;
                }
            },
            Nature::Fermeture | Nature::Survol => {
                if let Some(a) = registre().cherche(cle, nature) {
                    ajoute(a.bornes);
                }
            }
        }
    }
    for cle in &perdues[..n] {
        if let Some(a) = registre().annule(*cle, Nature::Ouverture) {
            retire(a);
        }
    }
    for b in a_recomposer().drain(..) {
        ajoute(b);
    }
    let actif = registre().actif();
    if !actif {
        if finies > 0 {
            mesure_et_oublie();
        }
        // Plus rien a animer : les tampons de travail rendent leur memoire.
        unsafe {
            let d = &mut *core::ptr::addr_of_mut!(DESSOUS);
            let s = &mut *core::ptr::addr_of_mut!(DESSUS);
            if d.capacity() != 0 || s.capacity() != 0 {
                *d = Vec::new();
                *s = Vec::new();
            }
        }
        // Registre vide : aucun instantane ne peut survivre a son animation.
        if !instantanes().is_empty() || unsafe { INSTANTANES_OCTETS } != 0 {
            crate::serial_println!("BOUCHAUD_ATELIER_INSTANTANE orphelins={} octets={}",
                instantanes().len(), unsafe { INSTANTANES_OCTETS });
            *instantanes() = Vec::new();
            unsafe { INSTANTANES_OCTETS = 0; }
        }
    }
    actif || zones > 0
}

/// Reste-t-il une animation a montrer ? (La boucle du bureau en fait une
/// echeance de trame.)
pub fn actif() -> bool {
    fil_ok() && registre().actif()
}

/// Debut d'une trame composee : fige l'instant de toutes les progressions.
pub fn debut_trame(maintenant: u64) {
    if !fil_ok() {
        return;
    }
    // Les animations lancees depuis la trame precedente commencent ICI : la
    // premiere image qu'on en voit est leur instant zero.
    registre().demarre(maintenant);
    EN_TRAME.store(true, core::sync::atomic::Ordering::Release);
    unsafe {
        HORLOGE_TRAME = maintenant;
        NS_TRAME = crate::kernel::timer::monotonic_ns();
        ANIME_CETTE_TRAME = false;
    }
}

/// Fin d'une trame composee : si elle a anime quelque chose, son cout compte.
pub fn fin_trame() {
    if !fil_ok() {
        return;
    }
    EN_TRAME.store(false, core::sync::atomic::Ordering::Release);
    unsafe {
        BUREAU_POSE = true;
        if MESURE_OUVERTURE {
            MESURE_OUVERTURE = false;
            let ns = crate::kernel::timer::monotonic_ns().saturating_sub(NS_TRAME);
            crate::serial_println!("BOUCHAUD_ATELIER_TRAME_OUVERTURE mode={:?} cout_us={}",
                mode(), ns / 1000);
        }
        if ANIME_CETTE_TRAME {
            let ns = crate::kernel::timer::monotonic_ns().saturating_sub(NS_TRAME);
            TRAMES_ANIMEES += 1;
            NS_ANIMES += ns;
            NS_PIRE = NS_PIRE.max(ns);
        }
    }
}

fn mesure_et_oublie() {
    unsafe {
        if TRAMES_ANIMEES > 0 {
            let (tas, _, _) = crate::kernel::heap::stats();
            crate::serial_println!(
                "BOUCHAUD_ATELIER_ANIMATION mode={:?} trames={} cout_moyen_us={} cout_max_us={} \
                 instantanes_pic_kio={} instantanes_kio={} tas_kio={}",
                mode(), TRAMES_ANIMEES, NS_ANIMES / TRAMES_ANIMEES as u64 / 1000, NS_PIRE / 1000,
                INSTANTANES_PIC / 1024, INSTANTANES_OCTETS / 1024, tas / 1024,
            );
        }
        INSTANTANES_PIC = INSTANTANES_OCTETS;
        TRAMES_ANIMEES = 0;
        NS_ANIMES = 0;
        NS_PIRE = 0;
    }
}

/// Cle du survol d'une cellule du dock : jamais un `WindowId` ni un
/// instantane.
pub fn cle_survol(cible: super::dock::Cible) -> u64 {
    (1 << 62) | match cible {
        super::dock::Cible::Applications => 0,
        super::dock::Cible::Fenetre(i) => i as u64 + 1,
    }
}

/// Une cellule prend le survol : fondu de `zone` (cellule et infobulle).
/// En mode `Aucun`, rien n'est lance : le survol s'affiche plein d'emblee.
pub fn survole(cible: super::dock::Cible, zone: Boite) {
    if !fil_ok() {
        return;
    }
    let duree = an::duree_ms(mode(), Nature::Survol);
    if duree == 0 {
        return;
    }
    if let Some(e) = registre().lance(Animation {
        cle: cle_survol(cible), nature: Nature::Survol, debut_ms: an::EN_ATTENTE,
        duree_ms: duree, depart: 0, bornes: zone,
    }) {
        retire(e);
    }
}

/// Le survol quitte une cellule : son fondu, s'il courait, s'arrete.
pub fn quitte(cible: super::dock::Cible) {
    if !fil_ok() {
        return;
    }
    if let Some(a) = registre().annule(cle_survol(cible), Nature::Survol) {
        retire(a);
    }
}

/// Avancement du survol de `cible` a l'instant de la trame (PLEIN s'il n'y a
/// pas de fondu en cours : le survol est etabli).
pub fn survol(cible: super::dock::Cible) -> u32 {
    if !lecture_ok() {
        return super::mouvement::PLEIN;
    }
    match registre().cherche(cle_survol(cible), Nature::Survol) {
        Some(a) => {
            unsafe { ANIME_CETTE_TRAME = true; }
            a.progression(unsafe { HORLOGE_TRAME })
        }
        None => super::mouvement::PLEIN,
    }
}

/// Le mouvement autorise-t-il de deplacer quelque chose (levee, translation) ?
pub fn deplacement_permis() -> bool {
    mode() == an::Mode::Normal
}

/// L'ouverture en cours de la fenetre `cle`, a l'instant de la trame :
/// `(decalage vers le bas, opacite)`. `None` : la fenetre se peint normalement.
pub fn ouverture(cle: u64) -> Option<(i32, u8)> {
    if !lecture_ok() {
        return None;
    }
    let a = registre().cherche(cle, Nature::Ouverture)?;
    let t = unsafe { HORLOGE_TRAME };
    if t >= a.fin_ms() {
        return None;
    }
    let p = a.progression(t);
    Some((an::decalage(mode(), p), an::opacite(Nature::Ouverture, 0, p)))
}

/// Zone recouverte par l'ouverture de `cle` (bornes du calque pendant
/// l'animation, pour que la decoupe ne l'ecarte pas).
pub fn zone_ouverture_courante(peinte: Boite, dy: i32) -> Boite {
    zone_ouverture(peinte, dy)
}

/// Nombre d'instantanes en cours de fermeture.
pub fn nombre_instantanes() -> usize {
    if !lecture_ok() {
        return 0;
    }
    instantanes().len()
}

/// Bornes du calque de l'instantane `rang` (ombre comprise).
pub fn bornes_instantane(rang: usize) -> Option<Boite> {
    if !lecture_ok() {
        return None;
    }
    let i = instantanes().get(rang)?;
    Some(agrandit(i.cadre, crate::gui::disposition::DEBORD_OMBRE as i32))
}

/// Libere tout (le bureau s'en va).
pub fn oublie_tout() {
    if !fil_ok() {
        return;
    }
    unsafe {
        INSTANTANES_OCTETS = 0;
        *core::ptr::addr_of_mut!(DESSOUS) = Vec::new();
        *core::ptr::addr_of_mut!(DESSUS) = Vec::new();
    }
    *registre() = Registre::neuf();
    *instantanes() = Vec::new();
    *a_recomposer() = Vec::new();
}

// ─── Peinture ──────────────────────────────────────────────────────────────

/// Copie la zone `z` du tampon dans `vers` (rangee par rangee).
fn copie(z: Boite, vers: &mut Vec<u32>) {
    vers.clear();
    for y in z.y..z.y + z.h {
        vers.extend_from_slice(fb::ligne_mut(z.x as usize, y as usize, z.w as usize));
    }
}

/// Repose `depuis` dans la zone `z` du tampon.
fn pose(z: Boite, depuis: &[u32]) {
    for (r, y) in (z.y..z.y + z.h).enumerate() {
        let ligne = fb::ligne_mut(z.x as usize, y as usize, z.w as usize);
        let n = ligne.len().min(z.w as usize);
        ligne[..n].copy_from_slice(&depuis[r * z.w as usize..r * z.w as usize + n]);
    }
}

#[inline]
fn melange(dessus: u32, dessous: u32, a: u32) -> u32 {
    if a >= 255 {
        return dessus & 0x00ff_ffff;
    }
    let inv = 255 - a;
    let r = (((dessus >> 16) & 0xff) * a + ((dessous >> 16) & 0xff) * inv) / 255;
    let g = (((dessus >> 8) & 0xff) * a + ((dessous >> 8) & 0xff) * inv) / 255;
    let b = ((dessus & 0xff) * a + (dessous & 0xff) * inv) / 255;
    (r << 16) | (g << 8) | b
}

fn decoupe() -> Boite {
    let (x0, y0, x1, y1) = fb::clip_rect();
    Boite { x: x0 as i32, y: y0 as i32, w: x1 as i32 - x0 as i32, h: y1 as i32 - y0 as i32 }
}

fn inter(a: Boite, b: Boite) -> Option<Boite> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    (x1 > x0 && y1 > y0).then_some(Boite { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// Compose une fenetre en cours d'ouverture dans la decoupe courante :
/// `peinte` ses bornes finales, `dy` son decalage, `alpha` son opacite.
pub fn compose_ouverture(peinte: Boite, dy: i32, alpha: u8, dessine: impl FnOnce()) {
    if !lecture_ok() {
        dessine();
        return;
    }
    // Opacite nulle : la fenetre n'est pas encore visible. La peindre puis la
    // melanger a zero ne changerait aucun pixel -- c'etait +11 ms sous TCG sur
    // la premiere trame, deja la plus chere d'une ouverture.
    if alpha == 0 {
        return;
    }
    let c = decoupe();
    let ecran = Boite { x: 0, y: 0, w: fb::width() as i32, h: fb::height() as i32 };
    // Pixels de destination : decoupe ∩ fenetre decalee. Pixels sources :
    // les memes, remontes de `dy` -- peut-etre au-dessus de la decoupe.
    let Some(dest) = inter(c, decale(peinte, dy)) else { return };
    let Some(source) = inter(decale(dest, -dy), ecran) else { return };
    unsafe { ANIME_CETTE_TRAME = true; }
    let dessous = unsafe { &mut *core::ptr::addr_of_mut!(DESSOUS) };
    let dessus = unsafe { &mut *core::ptr::addr_of_mut!(DESSUS) };
    copie(source, dessous);
    fb::set_clip(source.x as usize, source.y as usize, source.w as usize, source.h as usize);
    dessine();
    copie(source, dessus);
    pose(source, dessous);
    fb::set_clip(c.x as usize, c.y as usize, c.w as usize, c.h as usize);
    // La ou la fenetre n'a rien peint, dessus == dessous : on laisse le fond
    // (deja peint a sa place par les calques inferieurs) tel quel.
    let a = alpha as u32;
    let mut touches = 0u64;
    for y in dest.y..dest.y + dest.h {
        let sy = y - dy;
        if sy < source.y || sy >= source.y + source.h {
            continue;
        }
        let rangee = ((sy - source.y) * source.w) as usize;
        let ligne = fb::ligne_mut(dest.x as usize, y as usize, dest.w as usize);
        for (i, px) in ligne.iter_mut().enumerate() {
            let sx = dest.x + i as i32 - source.x;
            if sx < 0 || sx >= source.w {
                continue;
            }
            let k = rangee + sx as usize;
            let (s, d) = (dessus[k], dessous[k]);
            if s != d {
                *px = melange(s, *px, a);
                touches += 1;
            }
        }
    }
    fb::note_pixels_dessines(touches);
}

/// Le pixel `(x, y)` d'un rectangle `w x h` aux coins de rayon `r` est-il
/// dans la forme ?
fn dans_arrondi(x: i32, y: i32, w: i32, h: i32, r: i32) -> bool {
    let cx = if x < r { r - 1 - x } else if x >= w - r { x - (w - r) } else { return true };
    let cy = if y < r { r - 1 - y } else if y >= h - r { y - (h - r) } else { return true };
    cx * cx + cy * cy <= r * r
}

/// Peint l'instantane `rang` a l'instant de la trame, dans la decoupe.
pub fn dessine_instantane(rang: usize) {
    if !lecture_ok() {
        return;
    }
    let Some(i) = instantanes().get(rang) else { return };
    let Some(a) = registre().cherche(i.cle, Nature::Fermeture) else { return };
    let t = unsafe { HORLOGE_TRAME };
    let p = a.progression(t);
    let alpha = an::opacite(Nature::Fermeture, a.depart, p);
    if alpha == 0 {
        return;
    }
    unsafe { ANIME_CETTE_TRAME = true; }
    // Reduction autour du centre, en virgule fixe 16.16 (aucun flottant par
    // pixel : sous TCG, il couterait des secondes).
    let e = an::echelle(mode(), p).max(1);
    let (cw, ch) = (i.cadre.w, i.cadre.h);
    let w = (cw as u32 * e / 1000) as i32;
    let h = (ch as u32 * e / 1000) as i32;
    let cible = Boite { x: i.cadre.x + (cw - w) / 2, y: i.cadre.y + (ch - h) / 2, w, h };
    // L'ombre s'efface avec la fenetre, au lieu de disparaitre d'un coup.
    let ombre = (crate::gui::atelier::jetons::OMBRE_FENETRE_ACTIVE as u32 * alpha as u32 / 255) as u8;
    super::matiere::ombre_douce(
        crate::gui::windowing::Rect::new(cible.x, cible.y, w.max(0) as u32, h.max(0) as u32),
        i.rayon, crate::gui::disposition::DEBORD_OMBRE, ombre);
    let Some(dest) = inter(decoupe(), cible) else { return };
    let pas_x = ((cw as u64) << 16) / w.max(1) as u64;
    let pas_y = ((ch as u64) << 16) / h.max(1) as u64;
    let a32 = alpha as u32;
    let mut touches = 0u64;
    for y in dest.y..dest.y + dest.h {
        let sy = ((((y - cible.y) as u64) * pas_y) >> 16).min(ch as u64 - 1) as usize;
        let ligne = fb::ligne_mut(dest.x as usize, y as usize, dest.w as usize);
        for (k, px) in ligne.iter_mut().enumerate() {
            let sx = ((((dest.x + k as i32 - cible.x) as u64) * pas_x) >> 16)
                .min(cw as u64 - 1) as usize;
            // Hors des coins arrondis, l'instantane contient ce qu'il y avait
            // derriere la fenetre : on ne le repeint pas.
            if !dans_arrondi(sx as i32, sy as i32, cw, ch, i.rayon as i32) {
                continue;
            }
            *px = melange(i.pixels[sy * cw as usize + sx], *px, a32);
            touches += 1;
        }
    }
    fb::note_pixels_dessines(touches);
}
