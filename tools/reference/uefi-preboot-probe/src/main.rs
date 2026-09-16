#![no_main]
#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use uefi::boot::LoadImageSource;
use uefi::fs::FileSystem;
use uefi::proto::console::gop::{BltOp, BltPixel, GraphicsOutput};
use uefi::proto::console::pointer::Pointer;
use uefi::proto::console::text::Input;
use uefi::proto::device_path::LoadedImageDevicePath;
use uefi::proto::unsafe_protocol;
use uefi::{boot, cstr16, entry, Handle, Status};

#[repr(C)]
#[unsafe_protocol("bd8c1056-9f36-44ec-92a8-a6337f817986")]
struct EdidActive {
    size_of_edid: u32,
    edid: *const u8,
}

fn read_edid(handle: Handle) -> Vec<u8> {
    let Ok(proto) = boot::open_protocol_exclusive::<EdidActive>(handle) else { return Vec::new() };
    if proto.size_of_edid == 0 || proto.size_of_edid > 4096 || proto.edid.is_null() { return Vec::new(); }
    unsafe { core::slice::from_raw_parts(proto.edid, proto.size_of_edid as usize).to_vec() }
}

fn edid_native(edid: &[u8]) -> Option<(usize, usize)> {
    const MAGIC: [u8; 8] = [0x00,0xff,0xff,0xff,0xff,0xff,0xff,0x00];
    if edid.len() < 72 || edid[0..8] != MAGIC { return None; }
    let d = &edid[54..72];
    if d[0] == 0 && d[1] == 0 { return None; }
    let h = d[2] as usize | (((d[4] >> 4) as usize) << 8);
    let v = d[5] as usize | (((d[7] >> 4) as usize) << 8);
    (h != 0 && v != 0).then_some((h, v))
}

// ===========================================================================
// BOUCHAUD_PREBOOT_SANS_TEXTE_V1 : le fond, et rien d'autre
// ===========================================================================
//
// Ce prechargeur a porte successivement deux choses, et les deux etaient
// mauvaises.
//
// D'abord un « B » bleu, fige pendant toute la phase de chargement. Ensuite,
// pour le remplacer, un ecran de progression dessine avec une police
// BITMAP 8x8 -- la seule qu'on puisse rendre ici, faute de rasteriseur
// TrueType avant ExitBootServices. Verdict de l'utilisateur, sans appel :
// « des caracteres de polices en pixels [...] non ca c'est non ».
//
// Il a raison, et pour une raison qui n'est pas qu'esthetique : le noyau, lui,
// a un atlas TrueType pre-rasterise a la compilation, et son ecran de
// demarrage -- titre, barre, ligne d'etat -- est deja beau. Faire preceder ce
// bel ecran d'une version pixelisee du MEME ecran ne montre rien de plus et
// abime ce qu'on voit en premier.
//
// Le prechargeur ne peint donc plus que le fond, exactement a la teinte du
// noyau, pour que le passage de relais ne clignote pas. Le premier texte
// affiche par la machine est celui du noyau, dans sa vraie police.

/// La teinte de `ecran_faute::FOND_DEMARRAGE`, a l'identique.
const FOND: BltPixel = BltPixel::new(0x0D, 0x11, 0x17);

fn run() -> uefi::Result {
    uefi::helpers::init()?;
    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;
    let edid = read_edid(gop_handle.clone());
    let native = edid_native(&edid);

    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle)?;
    let modes: Vec<_> = gop.modes().collect();

    let mut report = String::new();
    let _ = writeln!(report, "BOUCHAUD UEFI PREBOOT PROBE V2");
    let _ = writeln!(report, "edid.bytes={}", edid.len());
    let _ = writeln!(report, "edid.native={}", native.map(|(w,h)| alloc::format!("{}x{}", w, h)).unwrap_or_else(|| String::from("unavailable")));

    for (index, mode) in modes.iter().enumerate() {
        let info = mode.info();
        let (w, h) = info.resolution();
        let _ = writeln!(report, "gop.mode.{}={}x{} stride={} format={:?}", index, w, h, info.stride(), info.pixel_format());
    }

    let selected = native
        .and_then(|wanted| modes.iter().copied().find(|m| m.info().resolution() == wanted))
        .or_else(|| modes.iter().copied().filter(|m| { let (w,h)=m.info().resolution(); h != 0 && w.saturating_mul(9) == h.saturating_mul(16) }).max_by_key(|m| { let (w,h)=m.info().resolution(); w.saturating_mul(h) }))
        .or_else(|| modes.iter().copied().max_by_key(|m| { let (w,h)=m.info().resolution(); w.saturating_mul(h) }));

    if let Some(mode) = selected {
        let (w, h) = mode.info().resolution();
        if gop.set_mode(&mode).is_ok() { let _ = writeln!(report, "gop.selected={}x{}", w, h); }
        else { let _ = writeln!(report, "gop.selected=SET_MODE_FAILED"); }
    }

    let current = gop.current_mode_info();
    let (cw, ch) = current.resolution();
    let _ = writeln!(report, "gop.current={}x{} stride={} format={:?}", cw, ch, current.stride(), current.pixel_format());

    let pointer_count = boot::find_handles::<Pointer>().map(|v| v.len()).unwrap_or(0);
    let keyboard_count = boot::find_handles::<Input>().map(|v| v.len()).unwrap_or(0);
    let _ = writeln!(report, "firmware.pointer_handles={}", pointer_count);
    let _ = writeln!(report, "firmware.keyboard_handles={}", keyboard_count);
    let _ = writeln!(report, "note=written before ExitBootServices; kernel xHCI report follows after handoff");

    // LE FOND SEUL, ET APRES `set_mode`.
    //
    // Avant `set_mode` la resolution n'est pas encore la bonne : peindre
    // avant obligerait a tout repeindre, et l'utilisateur verrait la bascule.
    // Ce que le firmware a reellement donne reste ecrit dans le rapport
    // BOUCHAUD-PREBOOT.TXT -- il n'a pas besoin d'etre a l'ecran.
    let _ = gop.blt(BltOp::VideoFill {
        color: FOND,
        dest: (0, 0),
        dims: (cw, ch),
    });

    let fs_proto = boot::get_image_file_system(boot::image_handle())?;
    let mut fs = FileSystem::new(fs_proto);
    let _ = fs.write(cstr16!("\\BOUCHAUD-PREBOOT.TXT"), report.as_bytes());

    let loader = match fs.read(cstr16!("\\EFI\\BOOT\\BOUCHAUD-LOADER.EFI")) {
        Ok(bytes) => bytes,
        Err(_) => {
            // L'ECHEC RESTE LISIBLE, MAIS PAR LE RAPPORT.
            //
            // Sans police ici, l'ecrire a l'ecran demanderait de reintroduire
            // exactement ce que ce lot retire. BOUCHAUD-PREBOOT.TXT vient
            // d'etre ecrit sur la cle, quelques lignes plus haut, et se relit
            // depuis Windows.
            return Err(Status::NOT_FOUND.into());
        }
    };
    drop(fs);

    // Le protocole se rend AVANT `start_image` : le chargeur enfant en a
    // besoin, et un GOP encore ouvert en exclusif le lui refuserait.
    drop(gop);

    // Preserve a full device path for the child image. The bootloader then
    // keeps a valid DeviceHandle and can reopen the same FAT to load
    // kernel-x86_64, boot.json and /ramdisk.
    let loaded_path = boot::open_protocol_exclusive::<LoadedImageDevicePath>(
        boot::image_handle(),
    )?;
    let child = boot::load_image(
        boot::image_handle(),
        LoadImageSource::FromBuffer {
            buffer: &loader,
            file_path: Some(&*loaded_path),
        },
    )?;
    drop(loaded_path);
    boot::start_image(child)
}

#[entry]
fn main() -> Status {
    match run() { Ok(()) => Status::SUCCESS, Err(error) => error.status() }
}
