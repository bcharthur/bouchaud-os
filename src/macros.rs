//! Macros d'affichage globales `print!` / `println!`.
//!
//! Elles ecrivent sur la sortie VGA texte via `drivers::vga::_print`. Les
//! equivalents serie `serial_print!` / `serial_println!` sont definis dans
//! `drivers::serial`.

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::drivers::vga::_print(format_args!($($arg)*))
    }};
}

#[macro_export]
macro_rules! println {
    () => {{
        $crate::print!("\n")
    }};
    ($fmt:expr) => {{
        $crate::print!(concat!($fmt, "\n"))
    }};
    ($fmt:expr, $($arg:tt)*) => {{
        $crate::print!(concat!($fmt, "\n"), $($arg)*)
    }};
}
