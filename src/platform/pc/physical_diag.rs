//! TRIGKEY physical bring-up diagnostics visible on the GOP framebuffer.
//!
//! V3.4 keeps visible bring-up and adds EP0 GET_REPORT fallback counters.
//! It validates the final path: interrupt endpoint -> Transfer Event -> report ->
//! Bouchaud keyboard/mouse bridge, including report-protocol HID discovery.
//! COM1 remains mirrored into RAM so exact xHCI completion codes stay visible.

use alloc::string::String;
use alloc::vec::Vec;

const USB_LIVE_MS: u64 = 8_000;
const LOG_PAGE_MS: u64 = 8_000;
const SMP_PAGE_MS: u64 = 8_000;
const MAX_LOG_LINES: usize = 55;
const MAX_COLS: usize = 118;

fn wait_until(deadline_ms: u64, poll_usb: bool) {
    while crate::kernel::timer::monotonic_ms() < deadline_ms {
        if poll_usb {
            crate::drivers::xhci_active::poll();
        }
        // PIT/HPET IRQ wakes us; on an active HID this also keeps the transfer
        // ring serviced without burning a full core in a spin loop.
        x86_64::instructions::hlt();
    }
}

fn ascii_line(input: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if out.len() >= max {
            break;
        }
        if ch == '\t' {
            out.push(' ');
        } else if ch.is_ascii_graphic() || ch == ' ' {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    out
}

fn trace_ascii() -> String {
    let raw = crate::drivers::serial::trace_snapshot();
    let mut out = String::with_capacity(raw.len());
    for byte in raw {
        match byte {
            b'\n' => out.push('\n'),
            b'\r' => {},
            b'\t' => out.push(' '),
            0x20..=0x7e => out.push(byte as char),
            _ => out.push('?'),
        }
    }
    out
}

fn relevant_usb(line: &str) -> bool {
    line.contains("XHCI")
        || line.contains("xHCI")
        || line.contains("USB_")
        || line.contains("USB-")
        || line.contains("USB ")
        || line.contains("HID")
        || line.contains("PORTSC")
        || line.contains("ROOT_PORT")
        || line.contains("Enable Slot")
        || line.contains("Address Device")
        || line.contains("HID_REPORT")
        || line.contains("HID_EVENT")
        || line.contains("TRANSFER_FAIL")
        || line.contains("SET_PROTOCOL")
}

fn relevant_smp(line: &str) -> bool {
    line.contains("SMP4")
        || line.contains("SMP_")
        || line.contains("SMP-")
        || line.contains("SMP ")
        || line.contains("[SMP")
        || line.contains("AP_STARTED")
        || line.contains("AP_ENTRY")
        || line.contains("SIPI")
}

fn filtered_lines(predicate: fn(&str) -> bool) -> Vec<String> {
    let trace = trace_ascii();
    let mut lines = Vec::new();
    for line in trace.lines() {
        if predicate(line) {
            lines.push(ascii_line(line, MAX_COLS));
        }
    }
    lines
}

fn page(title: &str, subtitle: &str) {
    use crate::drivers::gfx;
    gfx::fill_rect(0, 0, gfx::width(), gfx::height(), gfx::C_BLACK);
    gfx::draw_text_scaled(24, 18, title, gfx::C_CYAN, 2);
    gfx::draw_text(24, 58, subtitle, gfx::C_GRAY);
    gfx::draw_text(24, 76, "No keyboard/mouse required - pages advance automatically", gfx::C_DKGRAY);
}

fn kv(y: usize, label: &str, value: &str, color: u8) {
    use crate::drivers::gfx;
    gfx::draw_text(42, y, label, gfx::C_GRAY);
    gfx::draw_text(330, y, value, color);
}

fn usb_stop_reason(
    active: bool,
    ports: usize,
    enabled: usize,
    addressed: usize,
    control_ok: usize,
    devices: usize,
    keyboards: usize,
    mice: usize,
    reports: usize,
    keyboard_reports: usize,
    mouse_reports: usize,
) -> &'static str {
    if !active {
        "STOP: xHCI controller bring-up / ownership / RUN"
    } else if ports == 0 {
        "STOP: root-port visibility / PP / PORTSC"
    } else if enabled == 0 {
        "STOP: root-port reset did not reach PED=1"
    } else if addressed == 0 {
        "STOP: Enable Slot / Address Device command"
    } else if control_ok == 0 || devices == 0 {
        "STOP: EP0 control transfer / GET_DESCRIPTOR"
    } else if keyboards == 0 || mice == 0 {
        "STOP: HID report descriptor/classification or hub topology"
    } else if reports == 0 {
        "STOP: HID Interrupt-IN transport (TRB / doorbell / Transfer Event)"
    } else if keyboard_reports == 0 || mouse_reports == 0 {
        "STOP: HID report decode / report-id / input bridge"
    } else {
        "INPUT GREEN: keyboard + mouse reports are reaching Bouchaud"
    }
}

fn draw_usb_live(elapsed_ms: u64) {
    use crate::drivers::gfx;
    let active = crate::drivers::xhci_active::is_active();
    let ports = crate::drivers::xhci_active::connected_ports();
    let enabled = crate::drivers::xhci_active::enabled_ports();
    let addressed = crate::drivers::xhci_active::addressed_devices();
    let control_ok = crate::drivers::xhci_active::control_transfers_ok();
    let devices = crate::drivers::xhci_active::usb_devices();
    let keyboards = crate::drivers::xhci_active::hid_keyboards();
    let mice = crate::drivers::xhci_active::hid_mice();
    let polling = crate::drivers::xhci_active::hid_polling();
    let (polls, events, reports, keyboard_reports, mouse_reports, errors, rearms, kicks) =
        crate::drivers::xhci_active::hid_transport_stats();
    let (control_polls, control_reports, control_fails) =
        crate::drivers::xhci_active::hid_control_fallback_stats();
    let (mx, my) = crate::drivers::mouse::pos();
    let buttons = crate::drivers::mouse::buttons();
    let kbd = crate::drivers::keyboard::stats();

    page(
        "BOUCHAUD TRIGKEY V3.4 - USB/HID LIVE",
        "Move the mouse and press a few keys during this page if HID is detected.",
    );
    kv(110, "xHCI active", if active { "YES" } else { "NO" }, if active { gfx::C_GREEN } else { gfx::C_RED });
    kv(132, "root ports connected", &alloc::format!("{}", ports), if ports > 0 { gfx::C_GREEN } else { gfx::C_RED });
    kv(154, "root ports enabled", &alloc::format!("{}", enabled), if enabled > 0 { gfx::C_GREEN } else { gfx::C_RED });
    kv(176, "Address Device OK", &alloc::format!("{}", addressed), if addressed > 0 { gfx::C_GREEN } else { gfx::C_YELLOW });
    kv(198, "EP0 control OK", &alloc::format!("{}", control_ok), if control_ok > 0 { gfx::C_GREEN } else { gfx::C_YELLOW });
    kv(220, "USB devices enumerated", &alloc::format!("{}", devices), if devices > 0 { gfx::C_GREEN } else { gfx::C_YELLOW });
    kv(242, "HID keyboards", &alloc::format!("{}", keyboards), if keyboards > 0 { gfx::C_GREEN } else { gfx::C_RED });
    kv(264, "HID mice", &alloc::format!("{}", mice), if mice > 0 { gfx::C_GREEN } else { gfx::C_RED });
    kv(286, "xHCI polling", if polling { "ON" } else { "OFF" }, if polling { gfx::C_GREEN } else { gfx::C_YELLOW });
    kv(308, "polls / transfer events", &alloc::format!("{} / {}", polls, events), if events > 0 { gfx::C_GREEN } else { gfx::C_YELLOW });
    kv(330, "HID reports K / M", &alloc::format!("{} total  K={} M={}", reports, keyboard_reports, mouse_reports), if keyboard_reports > 0 && mouse_reports > 0 { gfx::C_GREEN } else { gfx::C_YELLOW });
    kv(352, "errors / rearms / kicks", &alloc::format!("{} / {} / {}", errors, rearms, kicks), if errors == 0 { gfx::C_GREEN } else { gfx::C_YELLOW });
    kv(374, "EP0 GET_REPORT fallback", &alloc::format!("polls={} reports={} fails={}", control_polls, control_reports, control_fails), if control_reports > 0 { gfx::C_GREEN } else { gfx::C_YELLOW });
    kv(396, "mouse / keyboard", &alloc::format!("x={} y={} btn={:#x} keyev={} pending={} last={:#04x}", mx, my, buttons, kbd.irq, kbd.pending, kbd.last_scancode), gfx::C_WHITE);

    let reason = usb_stop_reason(
        active, ports, enabled, addressed, control_ok, devices, keyboards, mice,
        reports, keyboard_reports, mouse_reports,
    );
    gfx::draw_text_scaled(42, 432, reason, if keyboard_reports > 0 && mouse_reports > 0 { gfx::C_GREEN } else { gfx::C_YELLOW }, 1);
    gfx::draw_text(42, 466, "V3.4 INPUT-NOW: Interrupt-IN primary + mandatory HID GET_REPORT EP0 fallback", gfx::C_CYAN);
    gfx::draw_text(58, 488, "events=0 + control reports>0 -> usable input via EP0 while periodic endpoint is repaired", gfx::C_WHITE);
    gfx::draw_text(58, 506, "endpoint-state lines on page 2 show RUNNING/HALTED/ERROR for the periodic path", gfx::C_WHITE);
    gfx::draw_text(42, 544, &alloc::format!("live {} / {} ms trace={} bytes", elapsed_ms, USB_LIVE_MS, crate::drivers::serial::trace_total_bytes()), gfx::C_DKGRAY);
}

fn draw_log_page(title: &str, subtitle: &str, lines: &[String]) {
    use crate::drivers::gfx;
    page(title, subtitle);
    let start = lines.len().saturating_sub(MAX_LOG_LINES);
    let mut y = 108usize;
    if lines.is_empty() {
        gfx::draw_text(30, y, "No matching serial markers captured.", gfx::C_RED);
        gfx::draw_text(30, y + 20, "This itself means the expected bring-up path may not have executed.", gfx::C_YELLOW);
    } else {
        for line in &lines[start..] {
            let color = if line.contains("FAIL") || line.contains("PARTIAL") || line.contains("ECHEC") {
                gfx::C_RED
            } else if line.contains("GREEN") || line.contains("_OK") {
                gfx::C_GREEN
            } else if line.contains("ROOT_PORT") || line.contains("ENUM") || line.contains("HID") {
                gfx::C_YELLOW
            } else {
                gfx::C_WHITE
            };
            gfx::draw_text(24, y, line, color);
            y += 16;
            if y + 16 >= gfx::height() {
                break;
            }
        }
    }
}

/// USB/HID diagnostics before touching the SMP bootstrap. This ordering is
/// intentional: even if a physical AP bring-up regresses, the xHCI evidence
/// remains visible long enough to photograph.
pub fn show_usb() {
    use crate::drivers::gfx;
    gfx::enter();
    crate::serial_println!("BOUCHAUD_PHYSICAL_DIAG_V33_USB_BEGIN");

    let start = crate::kernel::timer::monotonic_ms();
    let deadline = start.saturating_add(USB_LIVE_MS);
    let mut next_redraw = start;
    while crate::kernel::timer::monotonic_ms() < deadline {
        crate::drivers::xhci_active::poll();
        let now = crate::kernel::timer::monotonic_ms();
        if now >= next_redraw {
            draw_usb_live(now.saturating_sub(start));
            gfx::present();
            next_redraw = now.saturating_add(200);
        }
        x86_64::instructions::hlt();
    }

    let lines = filtered_lines(relevant_usb);
    draw_log_page(
        "BOUCHAUD TRIGKEY V3.4 - xHCI/USB/HID LOGS",
        "Latest relevant kernel serial markers captured in RAM.",
        &lines,
    );
    gfx::present();
    wait_until(crate::kernel::timer::monotonic_ms().saturating_add(LOG_PAGE_MS), true);
    crate::serial_println!("BOUCHAUD_PHYSICAL_DIAG_V33_USB_END lines={}", lines.len());
}

/// SMP page, called after the Stage2 path has actually executed `smp::init_probe`.
pub fn show_smp() {
    use crate::drivers::gfx;
    gfx::enter();
    let detected = crate::arch::x86_64::smp::discovered_cpus();
    let online = crate::arch::x86_64::smp::schedulable_cpus();
    let aps = crate::arch::x86_64::smp::started_aps();
    let lines = filtered_lines(relevant_smp);

    page(
        "BOUCHAUD TRIGKEY V3.4 - SMP PHYSICAL",
        "Stage2 now runs the real INIT/SIPI AP bootstrap; scheduler remains gated until after this page.",
    );
    kv(120, "logical CPUs detected", &alloc::format!("{}", detected), if detected > 1 { gfx::C_GREEN } else { gfx::C_YELLOW });
    kv(144, "CPUs online", &alloc::format!("{}", online), if online >= detected && detected > 1 { gfx::C_GREEN } else { gfx::C_RED });
    kv(168, "APs started", &alloc::format!("{} / {}", aps, detected.saturating_sub(1)), if aps >= detected.saturating_sub(1) && detected > 1 { gfx::C_GREEN } else { gfx::C_RED });
    kv(192, "scheduler enabled", if crate::arch::x86_64::smp::scheduler_enabled() { "YES" } else { "NO (gated for diagnostic)" }, gfx::C_YELLOW);

    let conclusion = if detected <= 1 {
        "STOP: CPU topology/discovery reports only BSP"
    } else if online < detected {
        "STOP: INIT/SIPI/trampoline/AP entry before ONLINE marker"
    } else {
        "SMP AP BOOT GREEN: scheduler will be enabled before desktop"
    };
    gfx::draw_text_scaled(42, 238, conclusion, if online >= detected && detected > 1 { gfx::C_GREEN } else { gfx::C_YELLOW }, 1);

    let start = lines.len().saturating_sub(38);
    let mut y = 290usize;
    for line in &lines[start..] {
        let color = if line.contains("PARTIAL") || line.contains("reason=") || line.contains("ECHEC") {
            gfx::C_RED
        } else if line.contains("GREEN") || line.contains("AP_STARTED") {
            gfx::C_GREEN
        } else {
            gfx::C_WHITE
        };
        gfx::draw_text(24, y, line, color);
        y += 16;
        if y + 16 >= gfx::height() { break; }
    }
    gfx::present();
    wait_until(crate::kernel::timer::monotonic_ms().saturating_add(SMP_PAGE_MS), false);
    crate::serial_println!(
        "BOUCHAUD_PHYSICAL_DIAG_V33_SMP_END detected={} online={} aps={}",
        detected, online, aps,
    );
}
