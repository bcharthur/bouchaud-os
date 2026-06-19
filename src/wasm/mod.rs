//! Mini-runtime WebAssembly de Bouchaud OS (no_std), bati sur l'interpreteur
//! `wasmi`. Permet d'executer des modules `.wasm` compiles depuis n'importe
//! quel langage (Rust, C, Zig, TinyGo, AssemblyScript...).
//!
//! Deux surfaces d'utilisation :
//!   - **OS natif** : `run_bytes(&wasm)` instancie le module, cable un petit ABI
//!     hote (`env.print*`) + un sous-ensemble WASI preview1 (`fd_write`,
//!     `proc_exit`, `random_get`, `clock_time_get`, `args/environ`...), appelle
//!     le point d'entree (`_start` / `main` / `run`) et renvoie la sortie.
//!     Utilise par la commande shell `wasm`.
//!   - **Web (API JS `WebAssembly`)** : `Module`/`Instance` persistants exposes
//!     a l'interpreteur JS (voir `gui::js`) : `WebAssembly.instantiate(bytes)`
//!     puis appel des fonctions exportees comme `instance.exports.add(2, 3)`.
//!
//! Securite : l'execution est bornee par du "fuel" (metrage d'instructions)
//! pour qu'un module en boucle infinie ne gele pas le noyau.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wasmi::core::ValueType;
use wasmi::{Caller, Config, Engine, Extern, ExternType, Linker, Memory, Module as WModule, Store, Value as WVal};

/// Budget d'instructions par invocation (anti-boucle infinie).
const FUEL: u64 = 200_000_000;

/// Etat hote partage avec le module pendant son execution.
pub struct HostState {
    /// Sortie texte accumulee (env.print*, WASI fd_write).
    pub out: String,
    /// Code de sortie demande via `proc_exit`, le cas echeant.
    pub exit_code: Option<i32>,
}

impl HostState {
    fn new() -> HostState {
        HostState { out: String::new(), exit_code: None }
    }
}

/// Resultat d'une execution complete (`run_bytes`).
pub struct RunResult {
    pub output: String,
    pub result: Option<i64>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

// ----------------------------------------------------------------------------
// Helpers memoire hote
// ----------------------------------------------------------------------------

fn caller_mem(caller: &mut Caller<'_, HostState>) -> Option<Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(m)) => Some(m),
        _ => None,
    }
}

fn read_bytes(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Option<Vec<u8>> {
    let mem = caller_mem(caller)?;
    let data = mem.data(&*caller);
    let start = ptr as usize;
    let end = start.checked_add(len.max(0) as usize)?;
    if end > data.len() {
        return None;
    }
    Some(data[start..end].to_vec())
}

fn write_mem(caller: &mut Caller<'_, HostState>, ptr: i32, bytes: &[u8]) {
    if let Some(mem) = caller_mem(caller) {
        let _ = mem.write(&mut *caller, ptr as usize, bytes);
    }
}

fn write_u32(caller: &mut Caller<'_, HostState>, ptr: i32, v: u32) {
    write_mem(caller, ptr, &v.to_le_bytes());
}

fn write_u64(caller: &mut Caller<'_, HostState>, ptr: i32, v: u64) {
    write_mem(caller, ptr, &v.to_le_bytes());
}

fn push_out(caller: &mut Caller<'_, HostState>, bytes: &[u8]) {
    match core::str::from_utf8(bytes) {
        Ok(s) => caller.data_mut().out.push_str(s),
        Err(_) => {
            for &b in bytes {
                caller.data_mut().out.push(b as char);
            }
        }
    }
}

// PRNG deterministe pour WASI random_get (suffisant pour un OS experimental).
fn prng_byte() -> u8 {
    use core::sync::atomic::{AtomicU64, Ordering};
    static S: AtomicU64 = AtomicU64::new(0x9E3779B97F4A7C15);
    let mut x = S.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    S.store(x, Ordering::Relaxed);
    (x >> 24) as u8
}

// ----------------------------------------------------------------------------
// WASI preview1 (sous-ensemble) + ABI "env"
// ----------------------------------------------------------------------------

fn wasi_fd_write(caller: &mut Caller<'_, HostState>, _fd: i32, iovs: i32, iovs_len: i32, nwritten: i32) -> i32 {
    let mem = match caller_mem(caller) {
        Some(m) => m,
        None => return 8, // EBADF-ish
    };
    let mut collected: Vec<u8> = Vec::new();
    let mut total: u32 = 0;
    {
        let data = mem.data(&*caller);
        let base = iovs.max(0) as usize;
        for k in 0..(iovs_len.max(0) as usize) {
            let rec = base + k * 8;
            if rec + 8 > data.len() {
                break;
            }
            let p = u32::from_le_bytes([data[rec], data[rec + 1], data[rec + 2], data[rec + 3]]) as usize;
            let l = u32::from_le_bytes([data[rec + 4], data[rec + 5], data[rec + 6], data[rec + 7]]) as usize;
            if p + l <= data.len() {
                collected.extend_from_slice(&data[p..p + l]);
                total += l as u32;
            }
        }
    }
    push_out(caller, &collected);
    write_u32(caller, nwritten, total);
    0
}

fn build_linker(engine: &Engine) -> Linker<HostState> {
    let mut l = Linker::new(engine);

    // --- ABI "env" minimal (modules custom / freestanding) ---
    l.func_wrap("env", "print", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
        if let Some(b) = read_bytes(&mut caller, ptr, len) {
            push_out(&mut caller, &b);
        }
    })
    .ok();
    l.func_wrap("env", "print_int", |mut caller: Caller<'_, HostState>, v: i32| {
        let s = format!("{}", v);
        caller.data_mut().out.push_str(&s);
    })
    .ok();
    l.func_wrap("env", "putchar", |mut caller: Caller<'_, HostState>, c: i32| {
        if let Some(ch) = core::char::from_u32(c as u32) {
            caller.data_mut().out.push(ch);
        }
    })
    .ok();

    // --- WASI preview1 (sous-ensemble suffisant pour "hello world") ---
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_write",
        |mut caller: Caller<'_, HostState>, fd: i32, iovs: i32, iovs_len: i32, nwritten: i32| -> i32 {
            wasi_fd_write(&mut caller, fd, iovs, iovs_len, nwritten)
        },
    )
    .ok();
    l.func_wrap("wasi_snapshot_preview1", "proc_exit", |mut caller: Caller<'_, HostState>, code: i32| {
        caller.data_mut().exit_code = Some(code);
    })
    .ok();
    l.func_wrap("wasi_snapshot_preview1", "fd_close", |_c: Caller<'_, HostState>, _fd: i32| -> i32 { 0 })
        .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_seek",
        |_c: Caller<'_, HostState>, _fd: i32, _off: i64, _whence: i32, _newoff: i32| -> i32 { 0 },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "fd_fdstat_get",
        |_c: Caller<'_, HostState>, _fd: i32, _buf: i32| -> i32 { 0 },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "environ_sizes_get",
        |mut caller: Caller<'_, HostState>, count_ptr: i32, size_ptr: i32| -> i32 {
            write_u32(&mut caller, count_ptr, 0);
            write_u32(&mut caller, size_ptr, 0);
            0
        },
    )
    .ok();
    l.func_wrap("wasi_snapshot_preview1", "environ_get", |_c: Caller<'_, HostState>, _a: i32, _b: i32| -> i32 { 0 })
        .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "args_sizes_get",
        |mut caller: Caller<'_, HostState>, count_ptr: i32, size_ptr: i32| -> i32 {
            write_u32(&mut caller, count_ptr, 0);
            write_u32(&mut caller, size_ptr, 0);
            0
        },
    )
    .ok();
    l.func_wrap("wasi_snapshot_preview1", "args_get", |_c: Caller<'_, HostState>, _a: i32, _b: i32| -> i32 { 0 })
        .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "random_get",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
            let n = len.max(0) as usize;
            let buf: Vec<u8> = (0..n).map(|_| prng_byte()).collect();
            write_mem(&mut caller, ptr, &buf);
            0
        },
    )
    .ok();
    l.func_wrap(
        "wasi_snapshot_preview1",
        "clock_time_get",
        |mut caller: Caller<'_, HostState>, _id: i32, _prec: i64, time_ptr: i32| -> i32 {
            let ns = (crate::kernel::timer::seconds() as u64).wrapping_mul(1_000_000_000);
            write_u64(&mut caller, time_ptr, ns);
            0
        },
    )
    .ok();

    l
}

// ----------------------------------------------------------------------------
// Conversions de valeurs
// ----------------------------------------------------------------------------

fn zero_val(t: &ValueType) -> WVal {
    match t {
        ValueType::I32 => WVal::I32(0),
        ValueType::I64 => WVal::I64(0),
        ValueType::F32 => WVal::F32(0.0f32.into()),
        ValueType::F64 => WVal::F64(0.0f64.into()),
        ValueType::FuncRef => WVal::FuncRef(wasmi::FuncRef::null()),
        ValueType::ExternRef => WVal::ExternRef(wasmi::ExternRef::null()),
    }
}

fn coerce(t: &ValueType, a: f64) -> WVal {
    match t {
        ValueType::I32 => WVal::I32(a as i64 as i32),
        ValueType::I64 => WVal::I64(a as i64),
        ValueType::F32 => WVal::F32((a as f32).into()),
        ValueType::F64 => WVal::F64(a.into()),
        ValueType::FuncRef => WVal::FuncRef(wasmi::FuncRef::null()),
        ValueType::ExternRef => WVal::ExternRef(wasmi::ExternRef::null()),
    }
}

fn wval_to_f64(v: &WVal) -> f64 {
    match v {
        WVal::I32(x) => *x as f64,
        WVal::I64(x) => *x as f64,
        WVal::F32(x) => f32::from(*x) as f64,
        WVal::F64(x) => f64::from(*x),
        _ => 0.0,
    }
}

fn wval_to_i64(v: &WVal) -> Option<i64> {
    Some(match v {
        WVal::I32(x) => *x as i64,
        WVal::I64(x) => *x,
        WVal::F32(x) => f32::from(*x) as i64,
        WVal::F64(x) => f64::from(*x) as i64,
        _ => return None,
    })
}

fn make_engine() -> Engine {
    let mut config = Config::default();
    config.consume_fuel(true);
    Engine::new(&config)
}

// ----------------------------------------------------------------------------
// Surface OS native : run_bytes
// ----------------------------------------------------------------------------

/// Instancie et execute un module `.wasm` : appelle `_start` / `main` / `run`,
/// renvoie la sortie texte et l'eventuel resultat numerique.
pub fn run_bytes(bytes: &[u8]) -> RunResult {
    let engine = make_engine();
    let module = match WModule::new(&engine, &bytes[..]) {
        Ok(m) => m,
        Err(e) => {
            return RunResult { output: String::new(), result: None, exit_code: None, error: Some(format!("module invalide : {}", e)) }
        }
    };
    let mut store = Store::new(&engine, HostState::new());
    let _ = store.add_fuel(FUEL);
    let linker = build_linker(&engine);
    let instance = match linker.instantiate(&mut store, &module).and_then(|pre| pre.start(&mut store)) {
        Ok(i) => i,
        Err(e) => {
            return RunResult { output: String::new(), result: None, exit_code: None, error: Some(format!("instanciation : {}", e)) }
        }
    };

    let mut result = None;
    let mut error = None;
    let entry = ["_start", "main", "run"].iter().find_map(|n| instance.get_func(&store, *n));
    if let Some(f) = entry {
        let ty = f.ty(&store);
        let params: Vec<WVal> = ty.params().iter().map(zero_val).collect();
        let mut results: Vec<WVal> = ty.results().iter().map(zero_val).collect();
        match f.call(&mut store, &params, &mut results) {
            Ok(()) => result = results.first().and_then(wval_to_i64),
            Err(e) => error = Some(format!("execution : {}", e)),
        }
    } else {
        error = Some("aucun point d'entree exporte (_start / main / run)".to_string());
    }

    let host = store.data();
    RunResult { output: host.out.clone(), result, exit_code: host.exit_code, error }
}

/// Valide un binaire WebAssembly (utilise par `WebAssembly.validate`). On parse
/// le module : un binaire invalide echoue, un binaire valide reussit.
pub fn validate(bytes: &[u8]) -> bool {
    let engine = make_engine();
    WModule::new(&engine, &bytes[..]).is_ok()
}

// ----------------------------------------------------------------------------
// Surface persistante : Instance (API JS WebAssembly)
// ----------------------------------------------------------------------------

/// Module WebAssembly instancie et persistant : conserve sa memoire lineaire et
/// son etat entre les appels. Expose la liste des fonctions exportees.
pub struct Instance {
    store: Store<HostState>,
    instance: wasmi::Instance,
    funcs: Vec<String>,
}

/// Instancie un module pour un usage persistant (appels repetes depuis JS).
pub fn instantiate(bytes: &[u8]) -> Result<Instance, String> {
    let engine = make_engine();
    let module = WModule::new(&engine, &bytes[..]).map_err(|e| format!("module invalide : {}", e))?;
    let funcs: Vec<String> = module
        .exports()
        .filter(|e| matches!(e.ty(), ExternType::Func(_)))
        .map(|e| e.name().to_string())
        .collect();
    let mut store = Store::new(&engine, HostState::new());
    let _ = store.add_fuel(FUEL);
    let linker = build_linker(&engine);
    let instance = linker
        .instantiate(&mut store, &module)
        .and_then(|pre| pre.start(&mut store))
        .map_err(|e| format!("instanciation : {}", e))?;
    Ok(Instance { store, instance, funcs })
}

impl Instance {
    /// Noms des fonctions exportees par le module.
    pub fn export_funcs(&self) -> &[String] {
        &self.funcs
    }

    /// Appelle une fonction exportee avec des arguments numeriques (coercion
    /// vers le type WASM attendu). Renvoie le premier resultat en `f64`.
    pub fn call(&mut self, name: &str, args: &[f64]) -> Result<Option<f64>, String> {
        let f = self
            .instance
            .get_func(&self.store, name)
            .ok_or_else(|| format!("fonction exportee absente : {}", name))?;
        // Recharge du fuel a chaque appel (chaque invocation a son budget).
        let _ = self.store.add_fuel(FUEL);
        let ty = f.ty(&self.store);
        let params: Vec<WVal> = ty
            .params()
            .iter()
            .enumerate()
            .map(|(i, t)| coerce(t, args.get(i).copied().unwrap_or(0.0)))
            .collect();
        let mut results: Vec<WVal> = ty.results().iter().map(zero_val).collect();
        f.call(&mut self.store, &params, &mut results).map_err(|e| format!("{}", e))?;
        Ok(results.first().map(wval_to_f64))
    }

    /// Sortie texte accumulee (env.print*, fd_write) depuis l'instanciation.
    pub fn output(&self) -> &str {
        &self.store.data().out
    }
}

// ----------------------------------------------------------------------------
// Selftest : module minimal genere a la main (pas de fichier requis)
// ----------------------------------------------------------------------------

/// Verifie la chaine complete parse -> validate -> instantiate -> call sur un
/// module exportant `add(i32, i32) -> i32` encode en dur.
pub fn selftest() -> Result<(), &'static str> {
    // (module (func (export "add") (param i32 i32) (result i32)
    //   local.get 0  local.get 1  i32.add))
    const ADD_WASM: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic + version
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f, // type: (i32,i32)->i32
        0x03, 0x02, 0x01, 0x00, // func: 1 fonction, type 0
        0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00, // export "add" func 0
        0x0a, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b, // code
    ];
    if !validate(ADD_WASM) {
        return Err("validation");
    }
    let mut inst = instantiate(ADD_WASM).map_err(|_| "instantiation")?;
    match inst.call("add", &[2.0, 3.0]) {
        Ok(Some(v)) if v == 5.0 => Ok(()),
        Ok(_) => Err("resultat"),
        Err(_) => Err("call"),
    }
}
