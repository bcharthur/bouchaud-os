//! Le systeme de fenetrage : geometrie, hit-test, politique, rasterisation.
//!
//! Ce fichier vivait sous `tests/`, ou `cargo test` ne peut pas le compiler :
//! la cible du projet est bare-metal, et lier le harnais standard fait entrer
//! deux `core` en conflit. Il ne s'executait donc jamais.
//!
//! Il suit desormais la convention du depot -- un binaire `rustc --test`
//! autonome qui inclut les modules par `#[path]` -- et tourne dans
//! `validate-fast`.
//!
//! Lance par `tools/gui/test-fenetrage.sh`.

extern crate alloc;
#[path="../../src/gui/windowing/mod.rs"] mod windowing;
mod gui { pub mod windowing { pub use crate::windowing::*; } }
#[path="../../src/gui/graphics.rs"] mod graphics;
#[path="../../src/gui/protocole.rs"] mod protocole;
#[path="../../src/gui/disposition.rs"] mod disposition;
use alloc::string::ToString;
use windowing::*;

/// LE PONT ENTRE LES DEUX GEOMETRIES.
///
/// `windowing` decrit ce que le peintre dessine (`painted_bounds`).
/// `disposition` decrit ce que le compositeur invalide
/// (`empreinte_fenetre_peinte`). Les deux modules ne se connaissent pas -- l'un
/// est le systeme de fenetrage, l'autre doit rester incluable sur l'hote sans
/// le noyau autour --, donc rien dans le typage ne les oblige a s'accorder.
///
/// Quand ils ont diverge, chaque fenetre deplacee a laisse deux bandes sombres
/// derriere elle : l'ombre est un ANNEAU autour du cadre, et l'invalidation ne
/// l'ajoutait qu'en bas et a droite. Ce test est le seul endroit du depot ou
/// les deux definitions se rencontrent.
#[test]
fn les_deux_geometries_de_l_ombre_coincident() {
    assert_eq!(manager::SHADOW_EXTENT, disposition::DEBORD_OMBRE,
        "le peintre et l'invalidation doivent partir du meme debord");
    for cadre in [Rect::new(0,0,40,30), Rect::new(100,100,200,150),
                  Rect::new(7,3,1,1), Rect::new(940,660,340,60)] {
        let peint = window_render_geometry(cadre, TITLEBAR_HEIGHT, WINDOW_RADIUS,
            manager::SHADOW_EXTENT).painted_bounds;
        let invalide = disposition::empreinte_fenetre_peinte(
            protocole::Rect::neuf(cadre.x, cadre.y, cadre.width, cadre.height));
        assert_eq!((peint.x, peint.y, peint.width, peint.height),
            (invalide.x, invalide.y, invalide.largeur, invalide.hauteur),
            "painted_bounds != empreinte_fenetre_peinte pour {cadre:?}");
    }
}

fn manager() -> BouchaudWindowManager { BouchaudWindowManager::new(WorkArea(Rect::new(0, 24, 1000, 700))) }
fn create_with(m: &mut BouchaudWindowManager, rect: Rect, flags: WindowFlags) -> WindowId {
    m.create("Test".to_string(), rect, WindowConstraints { min_width: 160, min_height: 96 }, flags)
}
fn create(m: &mut BouchaudWindowManager, rect: Rect) -> WindowId { create_with(m, rect, WindowFlags::STANDARD) }

#[test] fn ids_and_focus_survive_middle_close() { let mut m=manager(); let a=create(&mut m,Rect::new(0,30,200,100)); let b=create(&mut m,Rect::new(20,40,200,100)); let c=create(&mut m,Rect::new(40,50,200,100)); m.apply(WindowCommand::Focus(c)); m.apply(WindowCommand::Close(b)); assert_eq!(m.window(a).unwrap().id,a); assert_eq!(m.window(c).unwrap().id,c); assert_eq!(m.focus(),Some(c)); assert_eq!(m.z_order(),&[a,c]); }
#[test] fn close_focused_damages_new_focus() { let mut m=manager(); let a=create(&mut m,Rect::new(0,30,200,100)); let b=create(&mut m,Rect::new(300,30,200,100)); m.apply(WindowCommand::Focus(b)); let t=m.apply(WindowCommand::Close(b)); assert_eq!(m.focus(),Some(a)); assert_eq!(t.damage.len(),2); assert_eq!(t.damage[1].0,BouchaudWindowManager::footprint(m.window(a).unwrap().rect())); }
#[test] fn focus_damages_old_and_new() { let mut m=manager(); let a=create(&mut m,Rect::new(0,30,200,100)); let b=create(&mut m,Rect::new(300,30,200,100)); m.apply(WindowCommand::Focus(a)); assert_eq!(m.apply(WindowCommand::Focus(b)).damage.len(),2); }
#[test] fn minimize_focused_transfers_focus() { let mut m=manager(); let a=create(&mut m,Rect::new(0,30,200,100)); let b=create(&mut m,Rect::new(300,30,200,100)); m.apply(WindowCommand::Focus(b)); let t=m.apply(WindowCommand::Minimize(b)); assert_eq!(m.focus(),Some(a)); assert!(m.window(b).unwrap().min); assert_eq!(t.damage.len(),3); }
#[test] fn minimize_restore_preserves_normal_placement() { placement_roundtrip(WindowPlacement::Normal); }
#[test] fn minimize_restore_preserves_maximized_placement() { placement_roundtrip(WindowPlacement::Maximized); }
#[test] fn minimize_restore_preserves_snapped_placement() { placement_roundtrip(WindowPlacement::SnappedLeft); }
fn placement_roundtrip(place:WindowPlacement) { let mut m=manager(); let id=create(&mut m,Rect::new(40,60,350,240)); match place { WindowPlacement::Maximized=>{m.apply(WindowCommand::Maximize(id));}, WindowPlacement::SnappedLeft=>{m.apply(WindowCommand::Snap(id,SnapZone::Left));}, _=>{} } m.apply(WindowCommand::Minimize(id)); m.apply(WindowCommand::Restore(id)); assert!(!m.window(id).unwrap().min); assert_eq!(m.window(id).unwrap().placement,place); }
#[test] fn fixed_surface_refuses_geometry_commands_without_damage() { let mut m=manager(); let id=create_with(&mut m,Rect::new(40,60,350,240),WindowFlags::FIXED_SURFACE); let before=m.window(id).unwrap().clone(); for c in [WindowCommand::Resize(id,Rect::new(0,24,900,600),ResizeEdge::Right),WindowCommand::Maximize(id),WindowCommand::Snap(id,SnapZone::Left)] { assert!(m.apply(c).damage.is_empty()); assert_eq!(m.window(id).unwrap(),&before); } }
#[test] fn non_closable_refuses_close() { let mut m=manager(); let mut flags=WindowFlags::STANDARD; flags.closable=false; let id=create_with(&mut m,Rect::new(0,30,200,100),flags); assert!(m.apply(WindowCommand::Close(id)).damage.is_empty()); assert!(m.window(id).is_some()); }
#[test] fn raise_keeps_unique_z_order() { let mut m=manager(); let a=create(&mut m,Rect::new(0,30,200,100)); let b=create(&mut m,Rect::new(20,40,200,100)); m.apply(WindowCommand::Raise(a)); assert_eq!(m.z_order(),&[b,a]); }
#[test] fn maximize_restores_exact_rect() { let mut m=manager(); let original=Rect::new(40,60,350,240); let id=create(&mut m,original); m.apply(WindowCommand::Maximize(id)); m.apply(WindowCommand::Restore(id)); assert_eq!(m.window(id).unwrap().rect(),original); }
#[test] fn snaps_exclude_shell_bars() { let mut m=manager(); let id=create(&mut m,Rect::new(40,60,350,240)); m.apply(WindowCommand::Snap(id,SnapZone::Left)); assert_eq!(m.window(id).unwrap().rect(),Rect::new(0,24,500,700)); m.apply(WindowCommand::Restore(id)); m.apply(WindowCommand::Snap(id,SnapZone::Right)); assert_eq!(m.window(id).unwrap().rect(),Rect::new(500,24,500,700)); }
#[test] fn resize_respects_constraints() { let mut m=manager(); let id=create(&mut m,Rect::new(40,60,350,240)); m.apply(WindowCommand::Resize(id,Rect::new(-30,900,2,3),ResizeEdge::SouthEast)); assert_eq!(m.window(id).unwrap().rect(),Rect::new(0,628,160,96)); }
#[test] fn move_damages_old_and_new_footprints() { let mut m=manager(); let id=create(&mut m,Rect::new(40,60,350,240)); let old=BouchaudWindowManager::footprint(m.window(id).unwrap().rect()); let t=m.apply(WindowCommand::Move(id,Point{x:200,y:200})); assert_eq!(t.damage[0].0,old); assert_eq!(t.damage.len(),2); }
#[test] fn hover_damage_is_button_local() { let mut m=manager(); let id=create(&mut m,Rect::new(100,100,400,300)); let t=m.apply(WindowCommand::Hover(id,Some(HitRegion::Close))); assert_eq!(t.damage,&[Damage(close_button_rect(m.window(id).unwrap().rect(),WINDOW_CHROME))]); let t=m.apply(WindowCommand::Hover(id,Some(HitRegion::Maximize))); assert_eq!(t.damage.len(),2); assert_eq!(t.damage[1].0,maximize_button_rect(m.window(id).unwrap().rect(),WINDOW_CHROME)); }
#[test] fn canonical_hit_regions_and_painter_rects_agree() { let r=Rect::new(100,100,400,300); for (region,rect) in [(HitRegion::Close,close_button_rect(r,WINDOW_CHROME)),(HitRegion::Maximize,maximize_button_rect(r,WINDOW_CHROME)),(HitRegion::Minimize,minimize_button_rect(r,WINDOW_CHROME))] { let p=Point{x:rect.x+20,y:rect.y+16}; assert_eq!(hit_test(r,p,WINDOW_CHROME,true),region); } let cases=[(Point{x:102,y:102},HitRegion::NorthWest),(Point{x:502,y:97},HitRegion::NorthEast),(Point{x:102,y:398},HitRegion::SouthWest),(Point{x:498,y:398},HitRegion::SouthEast),(Point{x:102,y:200},HitRegion::Left),(Point{x:498,y:200},HitRegion::Right),(Point{x:200,y:102},HitRegion::Top),(Point{x:200,y:398},HitRegion::Bottom),(Point{x:200,y:115},HitRegion::Titlebar),(Point{x:200,y:200},HitRegion::Client),(Point{x:50,y:50},HitRegion::Outside)]; for(c,want)in cases { assert_eq!(hit_test(r,c,WINDOW_CHROME,true),want); } }
#[test] fn client_geometry_preserves_1100_by_604() { let outer=outer_rect_for_client_size(Point{x:20,y:30},1100,604,TITLEBAR_HEIGHT); assert_eq!(client_rect(outer,TITLEBAR_HEIGHT),Rect::new(21,63,1100,604)); assert_eq!(window_render_geometry(outer,TITLEBAR_HEIGHT,WINDOW_RADIUS,manager::SHADOW_EXTENT).client,Rect::new(21,63,1100,604)); }
#[test] fn double_click_is_monotonic_targeted_and_bounded() { let mut m=manager(); let a=create(&mut m,Rect::new(0,30,200,100)); let b=create(&mut m,Rect::new(0,30,200,100)); let mut d=DoubleClickDetector::default(); assert!(!d.click(a,1,Point{x:10,y:10},100)); assert!(!d.click(b,1,Point{x:10,y:10},200)); assert!(d.click(b,1,Point{x:14,y:13},500)); assert!(!d.click(a,1,Point{x:10,y:10},1000)); assert!(!d.click(a,1,Point{x:20,y:20},1100)); }
#[test] fn rounded_rasterizer_visits_only_damage_intersection() { let mut painted=0; let visited=graphics::fill_rounded_rect(Rect::new(0,0,1000,700),10,Rect::new(300,200,20,20),|_,_|painted+=1); assert_eq!(visited,400); assert_eq!(painted,400); }
#[test] fn rounded_rasterizer_never_paints_outside_clip() { let clip=Rect::new(10,10,5,5); graphics::stroke_rounded_rect(Rect::new(-20,-20,50,50),8,2,clip,|x,y|assert!(clip.contains(Point{x,y}))); }
#[test] fn shadow_footprint_uses_painter_extent() { assert_eq!(manager::SHADOW_EXTENT,8); assert_eq!(BouchaudWindowManager::footprint(Rect::new(10,20,100,50)),Rect::new(2,12,116,66)); }

// This model oracle proves transition geometry, footprints, focus, and hover
// rectangles. It intentionally does not claim to exercise widgets.rs, fontdue,
// the real framebuffer/shadows, the event loop, QEMU, or a Ladybird process.
fn render(m:&BouchaudWindowManager)->Vec<u8> {
    const W:usize=1000; const H:usize=724;
    let mut pixels=vec![0u8;W*H];
    for id in m.z_order() {
        let Some(window)=m.window(*id) else {continue}; if window.min {continue}
        paint(&mut pixels,BouchaudWindowManager::footprint(window.rect()),1);
        paint(&mut pixels,window.rect(),if m.focus()==Some(*id){4}else{2});
        if let Some((hover_id,region))=m.hover() { if hover_id==*id { let rect=match region {HitRegion::Close=>Some(close_button_rect(window.rect(),WINDOW_CHROME)),HitRegion::Maximize=>Some(maximize_button_rect(window.rect(),WINDOW_CHROME)),HitRegion::Minimize=>Some(minimize_button_rect(window.rect(),WINDOW_CHROME)),_=>None}; if let Some(rect)=rect{paint(&mut pixels,rect,7)} } }
    }
    pixels
}
fn paint(pixels:&mut[u8],rect:Rect,color:u8){const W:i32=1000;const H:i32=724;for y in rect.y.max(0)..rect.bottom().min(H){for x in rect.x.max(0)..rect.right().min(W){pixels[y as usize*W as usize+x as usize]=color}}}
fn oracle(m:&mut BouchaudWindowManager,command:WindowCommand){const W:usize=1000;let before=render(m);let transition=m.apply(command);let reference=render(m);let mut partial=before;for Damage(rect) in transition.damage{for y in rect.y.max(0)..rect.bottom().min(724){for x in rect.x.max(0)..rect.right().min(1000){partial[y as usize*W+x as usize]=reference[y as usize*W+x as usize]}}}assert_eq!(partial,reference);}
#[test] fn full_vs_partial_transition_oracle() { let mut m=manager();let a=create(&mut m,Rect::new(40,60,250,180));let b=create(&mut m,Rect::new(360,80,260,190));oracle(&mut m,WindowCommand::Focus(a));oracle(&mut m,WindowCommand::Focus(b));oracle(&mut m,WindowCommand::Hover(b,Some(HitRegion::Close)));oracle(&mut m,WindowCommand::Hover(b,Some(HitRegion::Maximize)));oracle(&mut m,WindowCommand::Move(b,Point{x:500,y:200}));oracle(&mut m,WindowCommand::Resize(b,Rect::new(500,200,300,240),ResizeEdge::SouthEast));oracle(&mut m,WindowCommand::Maximize(b));oracle(&mut m,WindowCommand::Restore(b));oracle(&mut m,WindowCommand::Snap(b,SnapZone::Left));oracle(&mut m,WindowCommand::Restore(b));oracle(&mut m,WindowCommand::Minimize(b));oracle(&mut m,WindowCommand::Restore(b));oracle(&mut m,WindowCommand::Focus(b));oracle(&mut m,WindowCommand::Close(b));}

#[test] fn one_id_authority_never_collides_or_recycles() {
    let direct = WindowId::allocate();
    let mut first = manager();
    let a = create(&mut first, Rect::new(0, 30, 200, 100));
    let b = create(&mut first, Rect::new(220, 30, 200, 100));
    first.apply(WindowCommand::Close(a));
    let c = create(&mut first, Rect::new(440, 30, 200, 100));
    let mut second = manager();
    let d = create(&mut second, Rect::new(0, 30, 200, 100));
    let ids = [direct, a, b, c, d];
    for i in 0..ids.len() { for j in i + 1..ids.len() { assert_ne!(ids[i], ids[j]); } }
}

#[test] fn controls_win_over_resize_border_for_every_pixel() {
    let window = Rect::new(100, 100, 400, 300);
    let controls = [(HitRegion::Close, close_button_rect(window, WINDOW_CHROME)),
        (HitRegion::Maximize, maximize_button_rect(window, WINDOW_CHROME)),
        (HitRegion::Minimize, minimize_button_rect(window, WINDOW_CHROME))];
    for (expected, rect) in controls {
        for y in rect.y..rect.bottom() { for x in rect.x..rect.right() {
            let point = Point { x, y };
            assert_eq!(hit_test(window, point, WINDOW_CHROME, true), expected);
            assert_eq!(hit_test(window, point, WINDOW_CHROME, false), expected);
        } }
    }
}

#[test] fn fixed_surface_hit_test_never_returns_resize() {
    let window = Rect::new(100, 100, 400, 300);
    for y in 95..405 { for x in 95..505 {
        let region = hit_test(window, Point { x, y }, WINDOW_CHROME, false);
        assert!(!matches!(region, HitRegion::Left | HitRegion::Right | HitRegion::Top
            | HitRegion::Bottom | HitRegion::NorthWest | HitRegion::NorthEast
            | HitRegion::SouthWest | HitRegion::SouthEast));
    } }
}

#[test] fn move_from_snap_or_maximize_restores_normal_geometry() {
    for command in [WindowCommand::Snap(WindowId::allocate(), SnapZone::Left),
        WindowCommand::Snap(WindowId::allocate(), SnapZone::Right),
        WindowCommand::Maximize(WindowId::allocate())] {
        let mut m = manager();
        let id = create(&mut m, Rect::new(40, 60, 350, 240));
        let placement = match command { WindowCommand::Snap(_, zone) => WindowCommand::Snap(id, zone),
            WindowCommand::Maximize(_) => WindowCommand::Maximize(id), _ => unreachable!() };
        m.apply(placement);
        m.apply(WindowCommand::Move(id, Point { x: 200, y: 180 }));
        let window = m.window(id).unwrap();
        assert_eq!(window.placement, WindowPlacement::Normal);
        assert_eq!(window.rect(), Rect::new(200, 180, 350, 240));
        assert!(window.restore_rect.is_none());
    }
}

#[test] fn real_shadow_and_opaque_contract_are_bounded() {
    let outer = Rect::new(40, 30, 180, 120);
    let geometry = window_render_geometry(outer, TITLEBAR_HEIGHT, WINDOW_RADIUS,
        manager::SHADOW_EXTENT);
    let mut painted = vec![];
    graphics::paint_window_shape(geometry, WINDOW_RADIUS, manager::SHADOW_EXTENT,
        Rect::new(-100, -100, 500, 500), 2, 3,
        |x, y, _| painted.push(Point { x, y }));
    assert!(painted.iter().all(|point| geometry.painted_bounds.contains(*point)));
    for y in geometry.opaque.y..geometry.opaque.bottom() {
        for x in geometry.opaque.x..geometry.opaque.right() {
            assert!(painted.contains(&Point { x, y }), "opaque pixel not painted: {x},{y}");
        }
    }
    for corner in [Point{x:outer.x,y:outer.y}, Point{x:outer.right()-1,y:outer.y},
        Point{x:outer.x,y:outer.bottom()-1}, Point{x:outer.right()-1,y:outer.bottom()-1}] {
        assert!(!geometry.opaque.contains(corner));
    }
}

const REAL_W: usize = 1000;
const REAL_H: usize = 724;
fn real_render(m: &BouchaudWindowManager, buffer: &mut [u32], clip: Rect, culling: bool) {
    let start = if culling { m.z_order().iter().rposition(|id| {
        let w=m.window(*id).unwrap(); !w.min && contains_rect(
            window_render_geometry(w.rect(),TITLEBAR_HEIGHT,WINDOW_RADIUS,manager::SHADOW_EXTENT).opaque,clip)
    }).map(|i|i+1).unwrap_or(0) } else { 0 };
    if start == 0 {
        for y in clip.y.max(0)..clip.bottom().min(REAL_H as i32) { for x in clip.x.max(0)..clip.right().min(REAL_W as i32) {
            buffer[y as usize*REAL_W+x as usize]=((x as u32*17)^(y as u32*31))|0x100000;
        }}
    }
    for id in &m.z_order()[start.saturating_sub(1)..] {
        let w=m.window(*id).unwrap(); if w.min {continue}
        let geometry=window_render_geometry(w.rect(),TITLEBAR_HEIGHT,WINDOW_RADIUS,manager::SHADOW_EXTENT);
        let border=if m.focus()==Some(*id){0xeeeeee}else{0x777777};
        graphics::paint_window_shape(geometry,WINDOW_RADIUS,manager::SHADOW_EXTENT,clip,0x20252d,border,
            |x,y,color| put(buffer,x,y,color));
        fill_real(buffer,geometry.client,clip,0x334455);
        if let Some((hover_id,region))=m.hover() { if hover_id==*id {
            let r=match region {HitRegion::Close=>Some(close_button_rect(w.rect(),WINDOW_CHROME)),HitRegion::Maximize=>Some(maximize_button_rect(w.rect(),WINDOW_CHROME)),HitRegion::Minimize=>Some(minimize_button_rect(w.rect(),WINDOW_CHROME)),_=>None};
            if let Some(r)=r {fill_real(buffer,r,clip,0xaa2222)}
        }}
    }
}
fn put(buffer:&mut[u32],x:i32,y:i32,color:u32){if x>=0&&y>=0&&(x as usize)<REAL_W&&(y as usize)<REAL_H{buffer[y as usize*REAL_W+x as usize]=color}}
fn fill_real(buffer:&mut[u32],r:Rect,clip:Rect,color:u32){let x0=r.x.max(clip.x).max(0);let y0=r.y.max(clip.y).max(0);let x1=r.right().min(clip.right()).min(REAL_W as i32);let y1=r.bottom().min(clip.bottom()).min(REAL_H as i32);for y in y0..y1{for x in x0..x1{put(buffer,x,y,color)}}}
fn contains_rect(a:Rect,b:Rect)->bool{a.x<=b.x&&a.y<=b.y&&a.right()>=b.right()&&a.bottom()>=b.bottom()}
fn real_full(m:&BouchaudWindowManager,culling:bool)->Vec<u32>{let mut b=vec![0;REAL_W*REAL_H];real_render(m,&mut b,Rect::new(0,0,REAL_W as u32,REAL_H as u32),culling);b}
fn real_oracle(m:&mut BouchaudWindowManager,command:WindowCommand){let mut partial=real_full(m,false);let transition=m.apply(command);for Damage(clip) in transition.damage{real_render(m,&mut partial,clip,true)}let reference=real_full(m,false);assert_eq!(partial,reference)}

#[test] fn real_shape_partial_render_oracle_on_patterned_background() {
    let mut m=manager();let a=create(&mut m,Rect::new(70,70,300,230));let b=create(&mut m,Rect::new(220,130,340,260));
    real_oracle(&mut m,WindowCommand::Focus(a));real_oracle(&mut m,WindowCommand::Focus(b));
    real_oracle(&mut m,WindowCommand::Hover(b,Some(HitRegion::Close)));
    real_oracle(&mut m,WindowCommand::Move(b,Point{x:221,y:130}));
    real_oracle(&mut m,WindowCommand::Move(b,Point{x:420,y:260}));
    real_oracle(&mut m,WindowCommand::Resize(b,Rect::new(420,260,400,300),ResizeEdge::SouthEast));
    real_oracle(&mut m,WindowCommand::Maximize(b));real_oracle(&mut m,WindowCommand::Restore(b));
    real_oracle(&mut m,WindowCommand::Snap(b,SnapZone::Left));real_oracle(&mut m,WindowCommand::Restore(b));
    real_oracle(&mut m,WindowCommand::Snap(b,SnapZone::Right));real_oracle(&mut m,WindowCommand::Minimize(b));
    real_oracle(&mut m,WindowCommand::Restore(b));real_oracle(&mut m,WindowCommand::Close(b));
}

#[test] fn rounded_corner_culling_matches_no_culling_and_outer_falsification_fails() {
    let mut m=manager();let id=create(&mut m,Rect::new(100,100,300,220));
    let w=m.window(id).unwrap();let geometry=window_render_geometry(w.rect(),TITLEBAR_HEIGHT,WINDOW_RADIUS,manager::SHADOW_EXTENT);
    let corner=Rect::new(geometry.outer.x,geometry.outer.y,3,3);
    let mut culled=real_full(&m,false);real_render(&m,&mut culled,corner,true);
    let mut reference=real_full(&m,false);real_render(&m,&mut reference,corner,false);assert_eq!(culled,reference);
    assert!(!contains_rect(geometry.opaque,corner));
    assert!(contains_rect(geometry.outer,corner),"falsified outer opacity would incorrectly cull background");
}
// ─── Rasterisation par segments ────────────────────────────────────────────
//
// La forme arrondie a deux descriptions : `inside_rounded`, qui teste un pixel,
// et `bornes_ligne`, qui calcule les bornes d'une ligne. Le compositeur
// n'utilise que la seconde. Si elles divergent, la fenetre est deformee -- et
// aucun test de placement ne le verrait.

#[test]
fn les_deux_descriptions_de_la_forme_sont_identiques() {
    // Exhaustif sur les tailles et rayons qui comptent : les degenerescences
    // (rayon nul, rayon superieur a la moitie), les petites fenetres ou le
    // rayon est ecrete, et une taille realiste.
    for (largeur, hauteur) in [(1, 1), (2, 2), (7, 5), (20, 20), (21, 13), (40, 30), (120, 80)] {
        for radius in [0u32, 1, 2, 3, 7, 10, 40] {
            for y in 0..hauteur {
                assert!(
                    graphics::ligne_conforme(largeur, hauteur, radius as i32, y),
                    "forme divergente : {largeur}x{hauteur} rayon {radius} ligne {y}"
                );
            }
        }
    }
}

/// Le rayon effectif est ecrete a la moitie du plus petit cote : un rayon
/// absurde ne doit pas produire une forme vide ni deborder.
#[test]
fn un_rayon_demesure_est_ecrete_sans_casser_la_forme() {
    let mut lignes = 0;
    let couverts = graphics::spans_rounded_rect(
        Rect::new(0, 0, 20, 20), 500, Rect::new(0, 0, 20, 20),
        |_, _, largeur| { lignes += 1; assert!(largeur <= 20) },
    );
    assert_eq!(lignes, 20, "une ligne par rangee, aucune perdue");
    assert!(couverts > 0 && couverts <= 400);
}

/// LA propriete de cout : un segment par ligne, pas un appel par pixel.
///
/// C'est ce qui separe un bureau qui repond d'un bureau qui rame. Une fenetre
/// maximisee faisait 893 440 appels de fermeture par remplissage ; elle en fait
/// desormais un par ligne.
#[test]
fn le_remplissage_coute_une_operation_par_ligne() {
    let rect = Rect::new(0, 0, 1280, 698);
    let mut segments = 0usize;
    let couverts = graphics::spans_rounded_rect(rect, 10, rect,
        |_, _, _| segments += 1);

    assert_eq!(segments, 698, "un segment par ligne");
    assert!(
        couverts > 800_000,
        "et pourtant la surface entiere est couverte ({couverts} pixels)"
    );
    assert!(
        (couverts / segments) > 1_000,
        "chaque operation couvre plus de mille pixels : c'est tout le gain"
    );
}

/// Le contour ne balaie plus l'aire : au plus deux segments par ligne.
///
/// `paint_window_shape` l'appelle une fois par anneau d'ombre plus une fois
/// pour la bordure -- neuf fois pour une fenetre. A l'ancienne, neuf balayages
/// complets de 893 440 pixels, soit huit millions d'iterations par trame.
#[test]
fn le_contour_ne_balaie_plus_toute_l_aire() {
    let rect = Rect::new(0, 0, 1280, 698);
    let mut segments = 0usize;
    let couverts = graphics::spans_stroke_rounded_rect(rect, 10, 1, rect,
        |_, _, _| segments += 1);

    assert!(segments <= 698 * 2, "au plus deux segments par ligne ({segments})");
    assert!(
        couverts < 10_000,
        "un contour d'un pixel ne couvre que son perimetre ({couverts})"
    );
    // A l'ancienne : 1280 x 698 = 893 440 iterations pour ces memes pixels.
    assert!(
        segments * 500 < 1280 * 698,
        "le cout doit etre d'un autre ordre de grandeur que l'aire"
    );
}

/// Une decoupe etroite ne doit produire que les lignes qu'elle croise.
#[test]
fn une_decoupe_etroite_borne_le_nombre_de_segments() {
    let rect = Rect::new(0, 0, 1280, 698);
    let mut segments = 0usize;
    graphics::spans_rounded_rect(rect, 10, Rect::new(600, 300, 14, 22),
        |_, _, largeur| { segments += 1; assert!(largeur <= 14) });
    assert_eq!(segments, 22, "une par ligne de la decoupe, et rien de plus");
}

/// Le contour d'une forme sans rayon reste un cadre exact.
#[test]
fn un_contour_sans_rayon_est_un_cadre_exact() {
    let rect = Rect::new(0, 0, 10, 6);
    let mut peints = alloc::vec::Vec::new();
    graphics::stroke_rounded_rect(rect, 0, 1, rect, |x, y| peints.push((x, y)));
    // Perimetre d'un cadre 10x6 d'un pixel : 2*10 + 2*(6-2) = 28.
    assert_eq!(peints.len(), 28);
    for (x, y) in peints {
        assert!(x == 0 || x == 9 || y == 0 || y == 5, "({x},{y}) n'est pas sur le bord");
    }
}

// ===========================================================================
// BOUCHAUD_C13_PAS_DEUX_FOIS_LE_MEME_PIXEL_V1
// ===========================================================================
//
// `[GUI-DAMAGE]` annonce trois pixels DESSINES pour un pixel PRESENTE. Une des
// trois passes etait structurelle : le fond de fenetre etait peint sous la
// surface du client, qui le recouvrait integralement juste apres.
//
// Le resultat a l'ecran est rigoureusement identique avec et sans cette passe,
// donc aucune capture ne peut la reveler -- et aucune ne peut non plus
// reveler qu'on en a retire un pixel de trop. Ces tests sont le seul endroit
// ou la difference est visible.

/// Pixels du fond de fenetre, par couleur, pour une trame complete.
fn fond_peint(opaque: Option<Rect>) -> alloc::collections::BTreeSet<(i32, i32)> {
    let cadre = Rect::new(120, 80, 400, 300);
    let geometrie = window_render_geometry(cadre, TITLEBAR_HEIGHT, WINDOW_RADIUS,
        manager::SHADOW_EXTENT);
    let decoupe = Rect::new(0, 0, 1280, 720);
    let mut pixels = alloc::collections::BTreeSet::new();
    graphics::paint_window_shape_spans(geometrie, WINDOW_RADIUS, manager::SHADOW_EXTENT,
        decoupe, 0x171b21, 0x454c58, opaque, |x, y, largeur, couleur, _couverture| {
            if couleur != 0x171b21 { return }
            for dx in 0..largeur as i32 { pixels.insert((x + dx, y)); }
        });
    pixels
}

/// Tout ce qui n'est PAS le fond -- ombre et filet de bordure.
fn ornement_peint(opaque: Option<Rect>) -> alloc::vec::Vec<(i32, i32, u32, u8)> {
    let cadre = Rect::new(120, 80, 400, 300);
    let geometrie = window_render_geometry(cadre, TITLEBAR_HEIGHT, WINDOW_RADIUS,
        manager::SHADOW_EXTENT);
    let decoupe = Rect::new(0, 0, 1280, 720);
    let mut pixels = alloc::vec::Vec::new();
    graphics::paint_window_shape_spans(geometrie, WINDOW_RADIUS, manager::SHADOW_EXTENT,
        decoupe, 0x171b21, 0x454c58, opaque, |x, y, largeur, couleur, couverture| {
            if couleur == 0x171b21 { return }
            for dx in 0..largeur as i32 { pixels.push((x + dx, y, couleur, couverture)); }
        });
    pixels
}

/// La zone utile telle que `draw_window` la calcule pour un client.
fn zone_client() -> Rect {
    client_rect(Rect::new(120, 80, 400, 300), TITLEBAR_HEIGHT)
}

#[test]
fn l_exclusion_retire_exactement_la_zone_recouverte_et_rien_d_autre() {
    let zone = zone_client();
    let complet = fond_peint(None);
    let ampute = fond_peint(Some(zone));

    let dans_zone = |(x, y): &(i32, i32)| {
        *x >= zone.x && *x < zone.right() && *y >= zone.y && *y < zone.bottom()
    };

    assert!(
        !ampute.iter().any(dans_zone),
        "aucun pixel de fond ne doit etre peint sous la surface du client"
    );
    let attendu: alloc::collections::BTreeSet<(i32, i32)> =
        complet.iter().copied().filter(|p| !dans_zone(p)).collect();
    assert_eq!(
        ampute, attendu,
        "et TOUT le reste doit etre peint a l'identique -- un pixel de fond \
         manquant hors de la zone laisserait voir le bureau au travers"
    );
}

#[test]
fn l_exclusion_ne_touche_ni_a_l_ombre_ni_a_la_bordure() {
    // La garantie de securite du changement. Le filet de bordure et les
    // anneaux d'ombre ne passent pas par le fond ; si l'exclusion pouvait les
    // manger, la fenetre perdrait son cadre sans qu'aucun test de forme ne
    // s'en apercoive.
    assert_eq!(ornement_peint(Some(zone_client())), ornement_peint(None));
}

#[test]
fn l_economie_vaut_la_quasi_totalite_du_fond() {
    let zone = zone_client();
    let complet = fond_peint(None);
    let economie = complet.len() - fond_peint(Some(zone)).len();

    // Pas tout a fait l'aire du rectangle : la zone utile n'est retraite que
    // de `WINDOW_BORDER` (1 px) alors que les coins BAS sont arrondis sur
    // `WINDOW_RADIUS` (10 px). Quelques pixels de la zone tombent donc hors de
    // la forme et n'etaient de toute facon pas peints.
    let dans_zone = complet.iter().filter(|(x, y)| {
        *x >= zone.x && *x < zone.right() && *y >= zone.y && *y < zone.bottom()
    }).count();
    assert_eq!(economie, dans_zone, "on retire exactement le fond recouvert");
    assert!(
        economie < (zone.width * zone.height) as usize,
        "et strictement moins que l'aire du rectangle, a cause des coins bas"
    );

    // La mesure qui justifie le changement : sur une fenetre de 400x300, neuf
    // dixiemes du fond etaient peints pour etre immediatement recouverts.
    let proportion = economie * 100 / complet.len();
    assert!(
        proportion >= 85,
        "au moins 85 % du fond etait invisible, mesure {proportion} %"
    );
}

#[test]
fn une_exclusion_hors_de_la_forme_ne_change_rien() {
    assert_eq!(fond_peint(Some(Rect::new(2000, 2000, 50, 50))), fond_peint(None));
    assert_eq!(fond_peint(Some(Rect::new(0, 0, 0, 0))), fond_peint(None));
}

#[test]
fn l_exclusion_conserve_le_cout_par_ligne() {
    // La propriete qui rend le gain reel : couper une ligne en deux ne doit
    // pas la faire degenerer en un appel par pixel. Au plus deux segments par
    // ligne, comme le contour autour de son trou.
    let rect = Rect::new(0, 0, 1280, 698);
    let mut segments = 0usize;
    let couverts = graphics::spans_rounded_rect_sauf(rect, 10, rect,
        Rect::new(1, 40, 1278, 600), |_, _, _| segments += 1);
    assert!(segments <= 698 * 2, "au plus deux segments par ligne, obtenu {segments}");
    assert!(segments >= 698, "et aucune ligne perdue");
    assert!(couverts > 0);
    assert!(
        couverts < 200_000,
        "la grande majorite de l'aire n'est plus peinte ({couverts} pixels)"
    );
}

#[test]
fn une_exclusion_qui_couvre_tout_ne_peint_aucun_fond() {
    let rect = Rect::new(0, 0, 200, 150);
    let couverts = graphics::spans_rounded_rect_sauf(rect, 10, rect, rect,
        |_, _, _| panic!("aucun segment ne doit sortir"));
    assert_eq!(couverts, 0);
}
