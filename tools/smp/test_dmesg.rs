//! Le vrai journal et ses facades ; seule la primitive CLI/lock est remplacee
//! par un mutex hote. Les macros refusent toute impression sous son verrou.
extern crate alloc;
use std::cell::{Cell, RefCell};
use std::sync::Mutex;
thread_local! {
    static HELD: Cell<usize> = const { Cell::new(0) };
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
}
static TEST: Mutex<()> = Mutex::new(());

fn emit(args: core::fmt::Arguments) {
    use std::fmt::Write;
    HELD.with(|n| assert_eq!(n.get(), 0, "console sous verrou dmesg"));
    OUTPUT.with(|o| write!(o.borrow_mut(), "{args}").unwrap());
}
#[macro_export]
macro_rules! serial_println { ($($a:tt)*) => { $crate::emit(format_args!($($a)*)) }; }
#[macro_export]
macro_rules! print { ($($a:tt)*) => { $crate::emit(format_args!($($a)*)) }; }
#[macro_export]
macro_rules! println { ($($a:tt)*) => {{ $crate::emit(format_args!($($a)*)); $crate::emit(format_args!("\n")); }}; }

mod kernel { pub mod sync {
    pub struct SpinLockIrq<T>(std::sync::Mutex<T>);
    pub struct Guard<'a, T>(std::sync::MutexGuard<'a, T>);
    impl<T> SpinLockIrq<T> {
        pub const fn new(v: T) -> Self { Self(std::sync::Mutex::new(v)) }
        pub fn lock(&self) -> Guard<'_, T> {
            let g = self.0.lock().unwrap();
            crate::HELD.with(|n| n.set(n.get() + 1));
            Guard(g)
        }
    }
    impl<T> core::ops::Deref for Guard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T { &self.0 }
    }
    impl<T> core::ops::DerefMut for Guard<'_, T> {
        fn deref_mut(&mut self) -> &mut T { &mut self.0 }
    }
    impl<T> Drop for Guard<'_, T> {
        fn drop(&mut self) { crate::HELD.with(|n| n.set(n.get() - 1)); }
    }
}}
#[path = "../../src/kernel/debug/dmesg.rs"]
mod dmesg;

fn snapshot() -> String {
    OUTPUT.with(|o| o.borrow_mut().clear());
    dmesg::print();
    OUTPUT.with(|o| core::mem::take(&mut *o.borrow_mut()))
}

#[test]
fn wrap_preserves_the_last_64_lines_in_order() {
    let _g = TEST.lock().unwrap(); dmesg::init();
    for n in 0..200 { dmesg::log_fmt(format_args!("message-{n:03}")); }
    assert_eq!(dmesg::count(), 64);
    let expected: String = (136..200).enumerate()
        .map(|(i,n)| format!("[{i:02}] message-{n:03}\n")).collect();
    assert_eq!(snapshot(), expected);
}

#[test]
fn short_overwrite_has_no_tail_and_reset_does_not_expose_old_lines() {
    let _g = TEST.lock().unwrap(); dmesg::init();
    for _ in 0..64 { dmesg::log(&"x".repeat(200)); }
    assert!(snapshot().lines().all(|l| l.len() == 5 + 96));
    for _ in 0..64 { dmesg::log("y"); }
    assert!(snapshot().lines().all(|l| l.ends_with("] y")));
    dmesg::init();
    assert_eq!(dmesg::count(), 1);
    assert_eq!(snapshot(), "[00] dmesg: tampon circulaire initialise\n");
}

#[test]
fn concurrent_writers_and_snapshots_never_mix_records() {
    let _g = TEST.lock().unwrap(); dmesg::init();
    dmesg::log("initial");
    std::thread::scope(|s| {
        for cpu in 0..4 {
            s.spawn(move || {
                for seq in 0..2000 {
                    dmesg::log_fmt(format_args!("cpu={cpu} seq={seq:04} {}", char::from(b'A' + cpu).to_string().repeat(60)));
                }
            });
        }
        for _ in 0..300 {
            let view = snapshot();
            assert!(view.lines().count() <= 64);
            for (i,line) in view.lines().enumerate() {
                assert!(line.starts_with(&format!("[{i:02}] ")));
                let text = &line[5..];
                if !text.starts_with("cpu=") { continue; }
                let cpu = text.as_bytes()[4] - b'0';
                assert!(cpu < 4);
                assert_eq!(text.len(), 75);
                assert!(text[15..].bytes().all(|b| b == b'A' + cpu));
            }
        }
    });
    assert_eq!(dmesg::count(), 64);
}
