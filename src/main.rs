#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::write_volatile;

const VGA_BUFFER: usize = 0xb8000;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;
const COLOR_LIGHT_GREEN_ON_BLACK: u8 = 0x0A;
const COLOR_LIGHT_RED_ON_BLACK: u8 = 0x0C;
const COLOR_WHITE_ON_BLACK: u8 = 0x0F;

static mut CURSOR_ROW: usize = 0;
static mut CURSOR_COL: usize = 0;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    clear_screen();

    print_colored("Bouchaud OS\n", COLOR_LIGHT_GREEN_ON_BLACK);
    print("Kernel experimental from scratch en Rust no_std\n");
    print("Version: 0.1.0\n\n");
    print("Etat: boot OK, VGA text OK, panic handler OK\n");
    print("Prochaine etape: UART, GDT/IDT, interruptions, memoire\n\n");
    print("bouchaud-os> ");

    loop {
        x86_64_hlt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    print_colored("\n\n[KERNEL PANIC]\n", COLOR_LIGHT_RED_ON_BLACK);

    if let Some(location) = info.location() {
        print("Fichier: ");
        print(location.file());
        print("\nLigne: ");
        print_usize(location.line() as usize);
        print("\n");
    }

    loop {
        x86_64_hlt();
    }
}

fn clear_screen() {
    for row in 0..VGA_HEIGHT {
        for col in 0..VGA_WIDTH {
            write_cell(row, col, b' ', COLOR_WHITE_ON_BLACK);
        }
    }

    unsafe {
        CURSOR_ROW = 0;
        CURSOR_COL = 0;
    }
}

fn print(text: &str) {
    print_colored(text, COLOR_WHITE_ON_BLACK);
}

fn print_colored(text: &str, color: u8) {
    for byte in text.bytes() {
        put_byte(byte, color);
    }
}

fn print_usize(mut value: usize) {
    if value == 0 {
        put_byte(b'0', COLOR_WHITE_ON_BLACK);
        return;
    }

    let mut buffer = [0u8; 20];
    let mut i = 0;

    while value > 0 {
        buffer[i] = b'0' + (value % 10) as u8;
        value /= 10;
        i += 1;
    }

    while i > 0 {
        i -= 1;
        put_byte(buffer[i], COLOR_WHITE_ON_BLACK);
    }
}

fn put_byte(byte: u8, color: u8) {
    unsafe {
        match byte {
            b'\n' => new_line(),
            byte => {
                if CURSOR_COL >= VGA_WIDTH {
                    new_line();
                }

                write_cell(CURSOR_ROW, CURSOR_COL, byte, color);
                CURSOR_COL += 1;
            }
        }
    }
}

unsafe fn new_line() {
    CURSOR_COL = 0;

    if CURSOR_ROW + 1 >= VGA_HEIGHT {
        clear_screen();
    } else {
        CURSOR_ROW += 1;
    }
}

fn write_cell(row: usize, col: usize, byte: u8, color: u8) {
    let offset = (row * VGA_WIDTH + col) * 2;
    let character_ptr = (VGA_BUFFER + offset) as *mut u8;
    let color_ptr = (VGA_BUFFER + offset + 1) as *mut u8;

    unsafe {
        write_volatile(character_ptr, byte);
        write_volatile(color_ptr, color);
    }
}

#[inline(always)]
fn x86_64_hlt() {
    unsafe {
        core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}
