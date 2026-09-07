use bootloader::{BootConfig, UefiBoot};
use std::env;
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek};
use std::path::{Path, PathBuf};

const MIB: u64 = 1024 * 1024;
const FAT_MIN_HEADROOM: u64 = 64 * MIB;

fn round_up_mib(value: u64) -> u64 { ((value + MIB - 1) / MIB) * MIB }

fn copy_into_fat(dir: &fatfs::Dir<&File>, target: &str, source: &Path) -> Result<u64, Box<dyn Error>> {
    let mut input = File::open(source)?;
    let mut output = dir.create_file(target)?;
    output.truncate()?;
    Ok(io::copy(&mut input, &mut output)?)
}

fn create_large_uefi_fat(
    tftp: &Path,
    ramdisk: &Path,
    preboot_shim: Option<&Path>,
    out_fat: &Path,
) -> Result<(u64, u64), Box<dyn Error>> {
    let bootloader = tftp.join("bootloader");
    let kernel = tftp.join("kernel-x86_64");
    let config = tftp.join("boot.json");
    for path in [&bootloader, &kernel, &config, ramdisk] {
        if !path.is_file() { return Err(format!("source UEFI/FAT absente: {}", path.display()).into()); }
    }
    if let Some(shim) = preboot_shim {
        if !shim.is_file() { return Err(format!("preboot shim absent: {}", shim.display()).into()); }
    }

    let mut needed = fs::metadata(&bootloader)?.len()
        + fs::metadata(&kernel)?.len()
        + fs::metadata(&config)?.len()
        + fs::metadata(ramdisk)?.len();
    if let Some(shim) = preboot_shim { needed = needed.saturating_add(fs::metadata(shim)?.len()); }

    let proportional = (needed + 31) / 32;
    let headroom = FAT_MIN_HEADROOM.max(proportional);
    let fat_size = round_up_mib(needed.checked_add(headroom).ok_or("taille FAT overflow")?);

    let fat_file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(out_fat)?;
    fat_file.set_len(fat_size)?;
    let options = fatfs::FormatVolumeOptions::new()
        .fat_type(fatfs::FatType::Fat32)
        .bytes_per_cluster(4096)
        .volume_label(*b"BOUCHAUDOS!");
    fatfs::format_volume(&fat_file, options)?;
    let filesystem = fatfs::FileSystem::new(&fat_file, fatfs::FsOptions::new())?;

    {
        let root = filesystem.root_dir();
        let efi = root.create_dir("efi")?;
        let boot = efi.create_dir("boot")?;
        let mut copied = 0u64;
        if let Some(shim) = preboot_shim {
            copied += copy_into_fat(&boot, "bootx64.efi", shim)?;
            copied += copy_into_fat(&boot, "bouchaud-loader.efi", &bootloader)?;
        } else {
            copied += copy_into_fat(&boot, "bootx64.efi", &bootloader)?;
        }
        copied += copy_into_fat(&root, "kernel-x86_64", &kernel)?;
        copied += copy_into_fat(&root, "boot.json", &config)?;
        copied += copy_into_fat(&root, "ramdisk", ramdisk)?;
        if copied != needed {
            return Err(format!("copie FAT incomplete: copied={} expected={}", copied, needed).into());
        }
    }
    filesystem.unmount()?;
    Ok((fat_size, headroom))
}

fn create_gpt_disk(fat_image: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let mut disk = OpenOptions::new().create(true).truncate(true).read(true).write(true).open(output)?;
    let partition_size = fs::metadata(fat_image)?.len();
    let disk_size = partition_size.checked_add(1024 * 64).ok_or("taille GPT overflow")?;
    disk.set_len(disk_size)?;
    let mbr = gpt::mbr::ProtectiveMBR::with_lb_size(u32::try_from((disk_size / 512) - 1).unwrap_or(0xFF_FF_FF_FF));
    mbr.overwrite_lba0(&mut disk)?;
    let block_size = gpt::disk::LogicalBlockSize::Lb512;
    let mut table = gpt::GptConfig::new().writable(true).initialized(false).logical_block_size(block_size).create_from_device(Box::new(&mut disk), None)?;
    table.update_partitions(Default::default())?;
    let id = table.add_partition("boot", partition_size, gpt::partition_types::EFI, 0, None)?;
    let partition = table.partitions().get(&id).ok_or_else(|| io::Error::new(io::ErrorKind::Other, "partition EFI absente apres creation"))?;
    let start = partition.bytes_start(block_size).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("offset partition EFI invalide: {e:?}")))?;
    table.write()?;
    disk.seek(io::SeekFrom::Start(start))?;
    io::copy(&mut File::open(fat_image)?, &mut disk)?;
    Ok(())
}

fn create_large_ramdisk_image(image: &UefiBoot, ramdisk: &Path, preboot_shim: Option<&Path>, output: &Path) -> Result<(), Box<dyn Error>> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = tempfile::tempdir_in(parent)?;
    let tftp = temp.path().join("tftp");
    fs::create_dir_all(&tftp)?;
    image.create_pxe_tftp_folder(&tftp)?;
    let fat = temp.path().join("boot-fat-large.img");
    let (fat_size, headroom) = create_large_uefi_fat(&tftp, ramdisk, preboot_shim, &fat)?;
    println!("BOUCHAUD_UEFI_FAT_HEADROOM fat_bytes={} headroom_bytes={}", fat_size, headroom);
    create_gpt_disk(&fat, output)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os();
    let _program = args.next();
    let usage = "usage: image-builder <kernel-elf> <output-image> [min-width min-height] [ramdisk] [preboot-shim]";
    let kernel = PathBuf::from(args.next().ok_or(usage)?);
    let output = PathBuf::from(args.next().ok_or(usage)?);
    let rest: Vec<_> = args.collect();

    let (min_width, min_height, ramdisk, preboot_shim): (Option<u64>, Option<u64>, Option<PathBuf>, Option<PathBuf>) = match rest.as_slice() {
        [] => (None, None, None, None),
        [ramdisk] => (None, None, Some(PathBuf::from(ramdisk)), None),
        [width, height] => (Some(width.to_string_lossy().parse()?), Some(height.to_string_lossy().parse()?), None, None),
        [width, height, ramdisk] => (Some(width.to_string_lossy().parse()?), Some(height.to_string_lossy().parse()?), Some(PathBuf::from(ramdisk)), None),
        [width, height, ramdisk, shim] => (Some(width.to_string_lossy().parse()?), Some(height.to_string_lossy().parse()?), Some(PathBuf::from(ramdisk)), Some(PathBuf::from(shim))),
        _ => return Err(usage.into()),
    };

    if !kernel.is_file() { return Err(format!("kernel ELF introuvable: {}", kernel.display()).into()); }
    if let Some(path) = ramdisk.as_ref() { if !path.is_file() { return Err(format!("ramdisk introuvable: {}", path.display()).into()); } }
    if let Some(path) = preboot_shim.as_ref() { if !path.is_file() { return Err(format!("preboot shim introuvable: {}", path.display()).into()); } }
    if let Some(parent) = output.parent() { fs::create_dir_all(parent)?; }

    let mut config = BootConfig::default();
    config.serial_logging = false;
    config.frame_buffer_logging = true;

    if let (Some(width), Some(height)) = (min_width, min_height) {
        if width != 0 || height != 0 {
            if width == 0 || height == 0 { return Err("min-width et min-height doivent etre tous deux nuls ou non nuls".into()); }
            if preboot_shim.is_none() {
                config.frame_buffer.minimum_framebuffer_width = Some(width as _);
                config.frame_buffer.minimum_framebuffer_height = Some(height as _);
                println!("BOUCHAUD_UEFI_FB_REQUEST min={}x{}", width, height);
            } else {
                println!("BOUCHAUD_UEFI_FB_PREBOOT_OWNS_MODE fallback_min={}x{}", width, height);
            }
        }
    }

    let mut image = UefiBoot::new(&kernel);
    image.set_boot_config(&config);
    if let Some(path) = ramdisk.as_ref() {
        println!("BOUCHAUD_UEFI_RAMDISK_EMBED {}", path.display());
        if let Some(shim) = preboot_shim.as_ref() { println!("BOUCHAUD_UEFI_PREBOOT_SHIM {}", shim.display()); }
        create_large_ramdisk_image(&image, path, preboot_shim.as_deref(), &output)?;
    } else {
        image.create_disk_image(&output)?;
    }
    println!("BOUCHAUD_UEFI_IMAGE_OK {}", output.display());
    Ok(())
}
