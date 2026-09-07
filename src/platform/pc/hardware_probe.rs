//! Inventaire materiel structure du TRIGKEY.
//!
//! Sonde read-only. Rapports dans `/diagnostics/` (RAMFS) + marqueurs serie.
//! La persistance sur la cle USB viendra avec xHCI + USB Mass Storage.

use alloc::format;
use alloc::string::String;
use core::fmt::Write as _;

use crate::boot::{BootInfo, FramebufferInfo};

fn ensure_dir(
    fs: &mut crate::fs::ramfs::FileSystem,
    parent: usize,
    name: &str,
) -> Option<usize> {
    fs.find_child(parent, name)
        .or_else(|| fs.mkdir_at(parent, name).ok())
}

fn put(name: &str, text: &str) -> bool {
    let mut fs = crate::fs::ramfs::fs();
    let Some(dir) = ensure_dir(&mut fs, 0, "diagnostics") else { return false };
    let Ok(node) = fs.touch_at(dir, name) else { return false };
    fs.write_node_bytes(node, text.as_bytes())
}

fn driver_binding(dev: &crate::arch::x86_64::pci::PciDevice) -> &'static str {
    match (dev.vendor, dev.device, dev.class, dev.subclass, dev.prog_if) {
        (0x10EC, 0x8168, _, _, _) => "rtl8168: implemented, physical validation pending",
        (0x8086, 0x100E, _, _, _) => "e1000: implemented",
        (_, _, 0x0C, 0x03, 0x30) => "xhci: probe-only foundation; runtime driver missing",
        (_, _, 0x03, _, _) => "display: UEFI GOP firmware framebuffer",
        (_, _, 0x01, 0x08, _) => "nvme: detected, runtime driver missing",
        (_, _, 0x04, _, _) => "audio: disabled in physical Stage2",
        _ => "unbound / generic",
    }
}

fn time_progress_probe() -> (u64, u64, bool) {
    let before_ticks = crate::kernel::timer::ticks();
    let before_ms = crate::kernel::timer::monotonic_ms();
    for _ in 0..500_000usize {
        core::hint::spin_loop();
    }
    let after_ticks = crate::kernel::timer::ticks();
    let after_ms = crate::kernel::timer::monotonic_ms();
    let dt = after_ticks.wrapping_sub(before_ticks);
    let dm = after_ms.wrapping_sub(before_ms);
    (dt, dm, dt != 0 || dm != 0)
}

pub fn run(boot: &'static BootInfo, framebuffer: FramebufferInfo) {
    crate::serial_println!("BOUCHAUD_HWPROBE_BEGIN");

    let metrics = crate::platform::pc::reference_metrics::snapshot(boot, framebuffer);
    let acpi = crate::platform::pc::acpi_probe::probe(boot);
    if let Some(hpet) = acpi.hpet {
        let _ = crate::kernel::timer::install_hpet_source(hpet.base_address);
    }
    let xhci = crate::drivers::xhci_probe::probe();
    let xhci_active = crate::drivers::xhci_active::bring_up();
    let (tick_delta, ms_delta, time_progress) = time_progress_probe();
    crate::platform::pc::hardware_facts::install_base(
        boot, framebuffer, metrics.logical_cpus_reported, time_progress, acpi.hpet.is_some(),
    );
    crate::platform::pc::hardware_facts::install_xhci(
        xhci.is_some(), xhci_active.active, xhci_active.connected_ports,
    );

    let mut hardware = String::new();
    let _ = writeln!(hardware, "BOUCHAUD OS PHYSICAL HARDWARE PROBE V1");
    let _ = writeln!(hardware, "cpu.brand={}", metrics.cpu_brand);
    let _ = writeln!(hardware, "cpu.vendor={}", metrics.cpu_vendor);
    let _ = writeln!(hardware, "cpu.logical_reported={}", metrics.logical_cpus_reported);
    let _ = writeln!(hardware, "cpu.tsc_mhz={}", metrics.tsc_mhz.unwrap_or(0));
    let _ = writeln!(hardware, "memory.usable_bytes={}", metrics.usable_memory_bytes);
    let _ = writeln!(hardware, "memory.regions={}", boot.memory_regions.len());
    let _ = writeln!(
        hardware,
        "memory.phys_offset={:#x}",
        boot.physical_memory_offset.unwrap_or(0)
    );
    let _ = writeln!(hardware, "timer.ticks_delta={}", tick_delta);
    let _ = writeln!(hardware, "timer.ms_delta={}", ms_delta);
    let _ = writeln!(hardware, "timer.progress={}", time_progress);

    let mut display = String::new();
    let _ = writeln!(display, "DISPLAY / GOP");
    let _ = writeln!(display, "current.width={}", framebuffer.width);
    let _ = writeln!(display, "current.height={}", framebuffer.height);
    let _ = writeln!(display, "current.stride={}", framebuffer.stride);
    let _ = writeln!(display, "current.bytes_per_pixel={}", framebuffer.bytes_per_pixel);
    let _ = writeln!(display, "current.format={:?}", framebuffer.pixel_format);
    let _ = writeln!(display, "current.framebuffer_bytes={}", framebuffer.byte_len);
    let _ = writeln!(display, "gop.available_modes=NOT_EXPORTED_BY_BOOTLOADER");
    let _ = writeln!(display, "edid=NOT_EXPORTED_AFTER_EXIT_BOOT_SERVICES");
    let _ = writeln!(
        display,
        "next_step=UEFI preboot shim: GOP QueryMode + EDID Active before ExitBootServices"
    );

    let mut pci_report = String::new();
    let mut drivers = String::new();
    let mut pci_count = 0usize;
    let mut network_count = 0usize;
    let mut display_count = 0usize;

    crate::arch::x86_64::pci::parcours(&mut |dev| {
        pci_count += 1;
        if dev.class == 0x02 { network_count += 1; }
        if dev.class == 0x03 { display_count += 1; }

        let irq = crate::arch::x86_64::pci::interrupt_line(dev);
        let _ = write!(
            pci_report,
            "{:02x}:{:02x}.{} {:04x}:{:04x} class={:02x}:{:02x}:{:02x} irq={} vendor={} class_name={}",
            dev.bus, dev.slot, dev.func, dev.vendor, dev.device,
            dev.class, dev.subclass, dev.prog_if, irq,
            crate::arch::x86_64::pci::vendor_name(dev.vendor),
            crate::arch::x86_64::pci::class_name(dev.class, dev.subclass),
        );
        for index in 0..6u8 {
            let address = crate::arch::x86_64::pci::bar_decode(dev, index).adresse();
            if address != 0 {
                let _ = write!(pci_report, " BAR{}={:#x}", index, address);
            }
        }
        let _ = writeln!(pci_report);
        let _ = writeln!(
            drivers,
            "{:02x}:{:02x}.{} {:04x}:{:04x} -> {}",
            dev.bus, dev.slot, dev.func, dev.vendor, dev.device,
            driver_binding(dev)
        );
        true
    });

    let acpi_text = acpi.render();
    let usb_text = match &xhci {
        Some(info) => info.render(),
        None => String::from("xHCI absent ou BAR non lisible\n"),
    };

    let mut errors = String::new();
    if !time_progress {
        let _ = writeln!(
            errors,
            "ERROR timer: no observable monotonic/PIT progress during probe"
        );
    }
    if metrics.tsc_mhz.is_none() {
        let _ = writeln!(errors, "WARN timer: TSC frequency unavailable");
    }
    if boot.rsdp_address.is_none() {
        let _ = writeln!(errors, "WARN ACPI: RSDP unavailable");
    } else if !acpi.rsdp_checksum_ok {
        let _ = writeln!(errors, "WARN ACPI: RSDP checksum invalid");
    }
    if acpi.hpet.is_none() {
        let _ = writeln!(errors, "WARN timer: HPET table not found");
    }
    if let Some(info) = &xhci {
        let connected = info.ports.iter().filter(|p| p.connected).count();
        let _ = writeln!(
            errors,
            "INFO USB: xHCI found, {} connected root-port(s); HID/mass-storage runtime driver pending",
            connected
        );
    } else {
        let _ = writeln!(errors, "WARN USB: xHCI controller not readable");
    }
    if framebuffer.width < 1280 || framebuffer.height < 720 {
        let _ = writeln!(
            errors,
            "WARN display: GOP mode {}x{} is low-resolution",
            framebuffer.width, framebuffer.height
        );
    }
    let _ = writeln!(
        errors,
        "INFO diagnostics: USB persistent sink pending xHCI mass-storage transport"
    );

    let readme = "\
Bouchaud OS hardware diagnostics.\n\
These files currently live in RAMFS and can be read with `cat /diagnostics/<file>`.\n\
They are NOT yet persistent across power-off.\n\
Writing them to the boot USB requires xHCI + USB Mass Storage.\n";

    let boot_json = format!(
        "{{\n  \"probe_version\": 1,\n  \"pci_devices\": {},\n  \"network_devices\": {},\n  \"display_devices\": {},\n  \"framebuffer_width\": {},\n  \"framebuffer_height\": {},\n  \"tsc_mhz\": {},\n  \"timer_progress\": {},\n  \"acpi_rsdp\": {},\n  \"acpi_hpet\": {},\n  \"xhci\": {},\n  \"usb_persistent_sink\": false\n}}\n",
        pci_count,
        network_count,
        display_count,
        framebuffer.width,
        framebuffer.height,
        metrics.tsc_mhz.unwrap_or(0),
        time_progress,
        boot.rsdp_address.is_some(),
        acpi.hpet.is_some(),
        xhci.is_some(),
    );

    let mut ok = true;
    ok &= put("hardware.txt", &hardware);
    ok &= put("pci.txt", &pci_report);
    ok &= put("acpi.txt", &acpi_text);
    ok &= put("usb.txt", &usb_text);
    ok &= put("xhci-active.txt", &xhci_active.render());
    ok &= put("display.txt", &display);
    ok &= put("drivers.txt", &drivers);
    ok &= put("errors.txt", &errors);
    ok &= put("README.txt", readme);
    ok &= put("boot.json", &boot_json);

    crate::serial_println!(
        "BOUCHAUD_HWPROBE_CPU vendor={} logical={} tsc_mhz={}",
        metrics.cpu_vendor,
        metrics.logical_cpus_reported,
        metrics.tsc_mhz.unwrap_or(0)
    );
    crate::serial_println!(
        "BOUCHAUD_HWPROBE_DISPLAY current={}x{} stride={} format={:?}",
        framebuffer.width, framebuffer.height, framebuffer.stride, framebuffer.pixel_format
    );
    crate::serial_println!(
        "BOUCHAUD_HWPROBE_ACPI rsdp={} tables={} hpet={} mcfg={}",
        boot.rsdp_address.is_some() as u8,
        acpi.tables.len(),
        acpi.hpet.is_some() as u8,
        acpi.mcfg.len()
    );
    if let Some(info) = &xhci {
        crate::serial_println!(
            "BOUCHAUD_HWPROBE_XHCI {:04x}:{:04x} version={:#x} ports={} usbsts={:#x}",
            info.vendor, info.device, info.hci_version, info.max_ports, info.usbsts
        );
    } else {
        crate::serial_println!("BOUCHAUD_HWPROBE_XHCI absent");
    }
    crate::serial_println!(
        "BOUCHAUD_HWPROBE_TIMER progress={} ticks_delta={} ms_delta={}",
        time_progress as u8, tick_delta, ms_delta
    );
    crate::serial_println!(
        "BOUCHAUD_DIAGNOSTICS_RAMFS_READY ok={} path=/diagnostics",
        ok as u8
    );
    crate::serial_println!(
        "BOUCHAUD_USB_DIAGNOSTIC_SINK_PENDING reason=xhci-mass-storage-not-implemented"
    );
    crate::serial_println!("BOUCHAUD_HWPROBE_END");
}
