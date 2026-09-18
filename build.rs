//! Build-time assets for exception paths.
//!
//! A kernel fault cannot allocate or initialise a TTF rasterizer.  We rasterize
//! the bundled DejaVu Sans once on the build host and include fixed alpha
//! atlases in the kernel.  Fault rendering then remains lock-free and
//! allocation-free while keeping antialiased glyphs.

use std::{env, fs, path::PathBuf};

fn main() {
    const FONT_PATH: &str = "src/assets/fonts/DejaVuSans.ttf";
    println!("cargo:rerun-if-changed={FONT_PATH}");

    // LE TAMPON DE CONSTRUCTION DOIT SUIVRE LA VARIABLE.
    //
    // `blackbox::BUILD_COMMIT` lit `BOUCHAUD_BUILD_COMMIT` par `option_env!`,
    // donc a la COMPILATION. Sans cette ligne, cargo ne recompile pas quand
    // la variable change : l'image gardait la valeur figee a la premiere
    // construction. L'archive du 18 septembre porte « commit=inconnu » sur
    // une image construite avec la variable posee -- et une archive qui ne
    // sait pas de quel binaire elle vient ne prouve rien.
    println!("cargo:rerun-if-env-changed=BOUCHAUD_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=BOUCHAUD_BUILD_LOT");

    // ET SI PERSONNE NE POSE LA VARIABLE, ON LA TROUVE SOI-MEME.
    //
    // Deux archives physiques de suite portent « commit=inconnu » alors que
    // l'image venait d'un commit connu : le tampon dependait de ce que
    // l'appelant veuille bien exporter, et un `cargo build` a la main ne le
    // veut jamais. Une archive qui ignore de quel binaire elle vient ne
    // prouve rien -- c'est tout l'objet du tampon.
    //
    // `rustc-env` pose la variable POUR la compilation, donc `option_env!` la
    // voit quelle que soit la facon dont le noyau a ete construit.
    if env::var_os("BOUCHAUD_BUILD_COMMIT").is_none() {
        if let Some(commit) = commit_git() {
            println!("cargo:rustc-env=BOUCHAUD_BUILD_COMMIT={commit}");
        }
    }
    // Le tampon suit le HEAD : sans cela, deux commits d'affilee donneraient
    // la meme valeur.
    for chemin in [".git/HEAD", ".git/refs/heads"] {
        if std::path::Path::new(chemin).exists() {
            println!("cargo:rerun-if-changed={chemin}");
        }
    }

    let bytes = fs::read(FONT_PATH).expect("read bundled DejaVu Sans");
    let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
        .expect("parse bundled DejaVu Sans");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));

    for scale in 1usize..=6 {
        let cell_width = 8 * scale;
        let cell_height = 10 * scale;
        let mut atlas = vec![0u8; 95 * cell_width * cell_height];
        let baseline = 8 * scale;

        for ascii in 32u8..=126 {
            let (metrics, bitmap) = font.rasterize(ascii as char, 8.0 * scale as f32);
            let left = ((cell_width as isize - metrics.width as isize) / 2).max(0);
            let top = baseline as isize - metrics.height as isize - metrics.ymin as isize;
            let glyph = (ascii - 32) as usize;
            for source_y in 0..metrics.height {
                let target_y = top + source_y as isize;
                if !(0..cell_height as isize).contains(&target_y) { continue; }
                for source_x in 0..metrics.width {
                    let target_x = left + source_x as isize;
                    if !(0..cell_width as isize).contains(&target_x) { continue; }
                    let source = bitmap[source_y * metrics.width + source_x];
                    let target = glyph * cell_width * cell_height
                        + target_y as usize * cell_width + target_x as usize;
                    atlas[target] = atlas[target].max(source);
                }
            }
        }

        fs::write(output.join(format!("fault-font-{scale}.bin")), atlas)
            .expect("write exception font atlas");
    }
}

/// Le commit court, lu depuis git au moment de la construction.
///
/// Rend `None` hors d'un depot : une archive sans tampon vaut mieux qu'un
/// tampon faux.
fn commit_git() -> Option<String> {
    let sortie = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    if !sortie.status.success() {
        return None;
    }
    let texte = String::from_utf8(sortie.stdout).ok()?;
    let texte = texte.trim().to_string();
    if texte.is_empty() {
        None
    } else {
        Some(texte)
    }
}
