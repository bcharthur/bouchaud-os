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
