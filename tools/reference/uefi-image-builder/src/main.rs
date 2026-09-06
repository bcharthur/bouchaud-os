use bootloader::{BootConfig, UefiBoot};
use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os();
    let _program = args.next();

    let kernel = PathBuf::from(
        args.next()
            .ok_or("usage: image-builder <kernel-elf> <output-image>")?,
    );
    let output = PathBuf::from(
        args.next()
            .ok_or("usage: image-builder <kernel-elf> <output-image>")?,
    );

    if args.next().is_some() {
        return Err("usage: image-builder <kernel-elf> <output-image>".into());
    }
    if !kernel.is_file() {
        return Err(format!("kernel ELF introuvable: {}", kernel.display()).into());
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut config = BootConfig::default();
    config.serial_logging = true;
    // Le Lot 2A prouve le chemin UEFI par la serie. On garde l'ecran propre
    // pour que le Lot 2B puisse prouver notre propre écriture GOP.
    config.frame_buffer_logging = false;

    let mut image = UefiBoot::new(&kernel);
    image.set_boot_config(&config);
    image.create_disk_image(&output)?;

    println!("BOUCHAUD_UEFI_IMAGE_OK {}", output.display());
    Ok(())
}
