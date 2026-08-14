//! Execution d'un programme : du fichier ELF a la premiere instruction ring 3.
//!
//! Enchaine les couches : espace d'adressage neuf ([`vmm`]), chargement de
//! l'image ELF (et de son interpreteur si le binaire est dynamique), pile
//! initiale avec argv/envp/auxv, puis creation de la tache et bascule en ring 3.
//!
//! Le noyau ne fait aucune resolution de symbole : pour un binaire dynamique il
//! charge `ld.so` a une base separee, donne le controle a **son** point
//! d'entree, et c'est `ld.so` qui mappe les bibliotheques via `mmap` et saute
//! ensuite dans le programme. C'est exactement le contrat de Linux, ce qui
//! permet d'utiliser un `ld-musl-x86_64.so.1` non modifie.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::arch::x86_64::usermode::TrapFrame;
use crate::kernel::{elf, task, vmm};

/// Variables d'environnement fournies par defaut a tout programme.
///
/// `QT_QPA_PLATFORM=linuxfb` oriente Qt vers le backend framebuffer, seul
/// disponible ici (pas de X11 ni de Wayland) ; les chemins `/dev/fb0` et
/// `/dev/input/*` correspondent aux peripheriques exposes par `kernel::fd`.
pub fn default_environment() -> Vec<String> {
    let session = crate::users::session();
    let (width, height) = crate::drivers::gfx::resolution();
    alloc::vec![
        "PATH=/bin:/usr/bin:/usr/local/bin".to_string(),
        alloc::format!("HOME={}", session.home()),
        alloc::format!("USER={}", session.username()),
        "SHELL=/bin/sh".to_string(),
        "TERM=linux".to_string(),
        "LANG=C.UTF-8".to_string(),
        "LD_LIBRARY_PATH=/lib:/usr/lib".to_string(),
        "TMPDIR=/tmp".to_string(),
        "XDG_RUNTIME_DIR=/var/run".to_string(),

        // --- Qt -----------------------------------------------------------
        // linuxfb : le seul backend possible ici (ni X11, ni Wayland, ni DRM).
        // On lui impose la geometrie pour lui epargner une autodetection, et on
        // le laisse prendre la main sur le terminal.
        "QT_QPA_PLATFORM=linuxfb".to_string(),
        alloc::format!("QT_QPA_PLATFORM_PLUGIN_ARGS=fb=/dev/fb0:size={}x{}", width, height),
        "QT_QPA_FB_DRM=0".to_string(),
        "QT_QPA_FB_TTY=/dev/tty0".to_string(),
        "QT_QPA_FB_HIDECURSOR=0".to_string(),
        "QT_QPA_FONTDIR=/usr/share/fonts/truetype/dejavu".to_string(),
        "QT_QPA_EVDEV_KEYBOARD_PARAMETERS=/dev/input/event0:grab=0".to_string(),
        "QT_QPA_EVDEV_MOUSE_PARAMETERS=/dev/input/event1:grab=0".to_string(),
        // Sans fontconfig ni dbus dans le systeme, autant le dire tout de suite.
        "QT_NO_FONTCONFIG=1".to_string(),
        "QT_LOGGING_RULES=qt.qpa.*=false".to_string(),
        "DBUS_SESSION_BUS_ADDRESS=disabled:".to_string(),

        // --- SDL ----------------------------------------------------------
        "SDL_VIDEODRIVER=fbcon".to_string(),
        "SDL_FBDEV=/dev/fb0".to_string(),

        // --- Python -------------------------------------------------------
        "PYTHONHOME=/usr".to_string(),
        "PYTHONDONTWRITEBYTECODE=1".to_string(),
        "PYTHONUNBUFFERED=1".to_string(),
    ]
}

/// Environnement transmis a un programme lance depuis le shell.
///
/// Les valeurs par defaut ci-dessus decrivent la machine ; celles que
/// l'utilisateur a posees avec `export` les recouvrent. C'est l'ordre attendu :
/// sans lui, `export QT_QPA_PLATFORM=vnc` n'aurait aucun effet, la valeur
/// compilee gagnant toujours.
pub fn shell_environment() -> Vec<String> {
    let mut env = default_environment();
    for entree in crate::shell::exported() {
        let nom = match entree.find('=') {
            Some(position) => entree[..position + 1].to_string(),
            None => continue,
        };
        env.retain(|existante| !existante.starts_with(&nom));
        env.push(entree);
    }
    env
}

/// Lit un fichier du RAMFS.
fn read_file(path: &str, cwd: usize) -> Result<Vec<u8>, String> {
    let fs = crate::fs::ramfs::fs();
    match fs.resolve(path, cwd) {
        Some(node) if fs.nodes[node].kind == crate::fs::ramfs::NodeKind::File => {
            Ok(fs.nodes[node].content.clone())
        }
        Some(_) => Err(alloc::format!("{} : est un repertoire", path)),
        None => Err(alloc::format!("{} : fichier introuvable", path)),
    }
}

/// Charge et execute un programme ELF, et attend sa fin.
///
/// Renvoie le code de sortie du processus.
pub fn exec(path: &str, argv: &[String], envp: &[String], cwd: usize) -> Result<i32, String> {
    if !crate::arch::x86_64::usermode::ready() {
        return Err("user-mode non initialise (ring 3 indisponible)".to_string());
    }
    let data = read_file(path, cwd)?;
    exec_image(path, &data, argv, envp, cwd)
}

/// Charge un programme et lance sa premiere tache **sans attendre sa fin**.
///
/// Rend le pid. C'est ce que `lance_navigateur` emploie : le gestionnaire de
/// fenetres etant lui-meme une tache ([`task::run_noyau`]), il n'a plus besoin
/// de bloquer sur `exec` — et il ne le peut plus, puisqu'il doit continuer a
/// composer pendant que le programme tourne.
///
/// `prepare` recoit le processus neuf avant que sa pile ne soit construite : il
/// y installe les descripteurs herites (surface, canal du protocole GUI) et rend
/// les variables d'environnement qui les designent. L'ordre compte — les numeros
/// de descripteur doivent etre connus avant d'ecrire `envp` dans la pile.
pub fn lance_detache(
    path: &str,
    argv: &[String],
    envp: &[String],
    cwd: usize,
    prepare: &mut dyn FnMut(&mut task::Process) -> Vec<String>,
) -> Result<u32, String> {
    if !crate::arch::x86_64::usermode::ready() {
        return Err("user-mode non initialise (ring 3 indisponible)".to_string());
    }
    let data = read_file(path, cwd)?;
    let (process, task) = construit_tache(path, &data, argv, envp, cwd, Some(prepare))?;
    let pid = process.borrow().pid;
    task::register(task);
    Ok(pid)
}

/// Meme chose a partir d'une image ELF deja en memoire (autotest embarque).
pub fn exec_image(
    name: &str,
    data: &[u8],
    argv: &[String],
    envp: &[String],
    cwd: usize,
) -> Result<i32, String> {
    // Un `exec` synchrone bloque le fil qui l'appelle jusqu'a la fin du
    // programme. Depuis une tache — le bureau, par exemple, qui est un fil
    // noyau —, cela ecraserait le contexte du fil noyau appelant, qui est
    // unique. Refuser est la seule reponse : le message arrive dans le terminal
    // du bureau, la ou l'utilisateur a tape la commande, au lieu que la machine
    // parte en silence.
    if task::in_user_task() {
        return Err(alloc::format!(
            "{} : exec synchrone impossible depuis le bureau (lancer depuis le shell texte)",
            name
        ));
    }
    let (process, first) = construit_tache(name, data, argv, envp, cwd, None)?;
    let _ = &process;

    // Un programme qui ouvre /dev/fb0 fait basculer la carte en mode graphique ;
    // il faut rendre le mode texte au shell quand il se termine, sinon la
    // console redevient invisible.
    let graphics_before = crate::drivers::gfx::is_active();

    let code = task::run(first);

    if !graphics_before && crate::drivers::gfx::is_active() {
        crate::drivers::gfx::leave();
    }
    Ok(code)
}

/// Fabrique un processus, y charge l'image ELF, et rend sa premiere tache.
///
/// Ce qui suit etait le corps d'`exec_image`. Il en est sorti pour qu'un
/// lancement synchrone et un lancement detache partagent exactement le meme
/// chargeur : deux copies auraient diverge des le premier correctif d'`auxv`.
fn construit_tache(
    name: &str,
    data: &[u8],
    argv: &[String],
    envp: &[String],
    cwd: usize,
    prepare: Option<&mut dyn FnMut(&mut task::Process) -> Vec<String>>,
) -> Result<(alloc::rc::Rc<core::cell::RefCell<task::Process>>, alloc::boxed::Box<task::Task>), String> {
    let process = match task::new_process(name, cwd) {
        Some(process) => process,
        None => return Err("memoire physique insuffisante (espace d'adressage)".to_string()),
    };

    // Les descripteurs herites sont poses avant la construction de la pile :
    // l'environnement doit pouvoir citer leurs numeros.
    //
    // Les variables rendues par `prepare` **recouvrent** celles de base, elles
    // ne s'y ajoutent pas. La distinction n'est pas cosmetique : la libc rend la
    // premiere occurrence trouvee, donc une seconde definition de
    // `QT_QPA_PLATFORM_PLUGIN_ARGS` placee a la fin n'aurait aucun effet — Qt
    // aurait garde la taille de la dalle alors que sa surface fait celle d'une
    // fenetre, et aurait peint au-dela de ce qui lui est projete.
    let envp = match prepare {
        Some(prepare) => {
            let mut complet = envp.to_vec();
            for entree in prepare(&mut process.borrow_mut()) {
                let nom = match entree.find('=') {
                    Some(position) => entree[..position + 1].to_string(),
                    None => continue,
                };
                complet.retain(|existante| !existante.starts_with(&nom));
                complet.push(entree);
            }
            complet
        }
        None => envp.to_vec(),
    };
    let envp = &envp[..];

    let (entry, stack) = {
        let mut borrowed = process.borrow_mut();
        let borrowed = &mut *borrowed;

        let image = elf::load(&mut borrowed.space, data, vmm::user_load_base())
            .map_err(|message| alloc::format!("{} : {}", name, message))?;

        // Binaire dynamique : on charge aussi l'editeur de liens et on lui donne
        // la main. Le programme est deja en memoire, ld.so le trouvera via
        // AT_PHDR/AT_ENTRY.
        let (entry, interp_base) = match image.interp.as_deref() {
            None => (image.entry, 0),
            Some(interp_path) => {
                let interp_data = read_file(interp_path, cwd).map_err(|_| {
                    alloc::format!(
                        "{} : interpreteur {} absent du RAMFS (installer ld-musl-x86_64.so.1)",
                        name, interp_path
                    )
                })?;
                let interp = elf::load(&mut borrowed.space, &interp_data, vmm::user_interp_base())
                    .map_err(|message| alloc::format!("{} : {}", interp_path, message))?;
                (interp.entry, interp.base)
            }
        };

        // Le tas `brk` commence apres l'image, avec un espace de garde.
        borrowed.brk_start = (image.end + 0x10_0000) & !0xFFF;
        borrowed.brk = borrowed.brk_start;

        let layout = elf::StackLayout {
            argv,
            envp,
            image: &image,
            interp_base,
            uid: borrowed.uid,
            gid: borrowed.gid,
        };
        let stack = elf::build_stack(&mut borrowed.space, &layout)
            .map_err(|message| message.to_string())?;
        (entry, stack)
    };

    crate::kernel::dmesg::log_fmt(format_args!(
        "exec: {} entree={:#x} pile={:#x}",
        name, entry, stack
    ));

    let frame = TrapFrame::new_user(entry, stack);
    let first = task::Task::new(process.clone(), frame);
    Ok((process, first))
}

/// Petit programme ELF64 statique genere a la volee, entierement en langage
/// machine : il valide la chaine complete (chargement, ring 3, `syscall`,
/// `mmap`, retour) sans dependre d'une libc externe.
///
/// Il execute :
/// `write(1, msg)`, `mmap(anonyme 4 KiB)`, ecrit dans la page obtenue,
/// `write(1, page)`, `getpid()`, `exit_group(0)`.
pub fn selftest_image() -> Vec<u8> {
    let base: u64 = vmm::user_load_base();
    const EHDR: usize = 64;
    const PHDR: usize = 56;
    const CODE_OFF: usize = EHDR + PHDR;

    let message = b"[ring3] ecriture directe par syscall write\n";

    let mut code: Vec<u8> = Vec::new();
    // write(1, message, len) -- l'adresse du message est calculee plus bas.
    code.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]); // mov eax, 1 (write)
    code.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]); // mov edi, 1 (stdout)
    let lea_patch = code.len() + 3;
    code.extend_from_slice(&[0x48, 0x8D, 0x35, 0, 0, 0, 0]); // lea rsi, [rip+disp32]
    code.extend_from_slice(&[0xBA]); // mov edx, imm32
    code.extend_from_slice(&(message.len() as u32).to_le_bytes());
    code.extend_from_slice(&[0x0F, 0x05]); // syscall

    // mmap(NULL, 4096, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)
    code.extend_from_slice(&[0xB8, 0x09, 0x00, 0x00, 0x00]); // mov eax, 9
    code.extend_from_slice(&[0x31, 0xFF]); // xor edi, edi
    code.extend_from_slice(&[0xBE, 0x00, 0x10, 0x00, 0x00]); // mov esi, 4096
    code.extend_from_slice(&[0xBA, 0x03, 0x00, 0x00, 0x00]); // mov edx, 3
    code.extend_from_slice(&[0x41, 0xBA, 0x22, 0x00, 0x00, 0x00]); // mov r10d, 0x22
    code.extend_from_slice(&[0x49, 0xC7, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]); // mov r8, -1
    code.extend_from_slice(&[0x4D, 0x31, 0xC9]); // xor r9, r9
    code.extend_from_slice(&[0x0F, 0x05]); // syscall
    code.extend_from_slice(&[0x48, 0x89, 0xC3]); // mov rbx, rax (adresse mappee)

    // Ecrit "[ring3] page mmap ok\n" dans la page fraichement mappee.
    let mapped = b"[ring3] page mmap anonyme accessible en ecriture\n";
    for (index, byte) in mapped.iter().enumerate() {
        // mov byte ptr [rbx+index], imm8
        code.extend_from_slice(&[0xC6, 0x83]);
        code.extend_from_slice(&(index as u32).to_le_bytes());
        code.push(*byte);
    }
    code.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]); // mov eax, 1
    code.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]); // mov edi, 1
    code.extend_from_slice(&[0x48, 0x89, 0xDE]); // mov rsi, rbx
    code.extend_from_slice(&[0xBA]);
    code.extend_from_slice(&(mapped.len() as u32).to_le_bytes());
    code.extend_from_slice(&[0x0F, 0x05]); // syscall

    // getpid() puis exit_group(0).
    code.extend_from_slice(&[0xB8, 0x27, 0x00, 0x00, 0x00]); // mov eax, 39
    code.extend_from_slice(&[0x0F, 0x05]); // syscall
    code.extend_from_slice(&[0xB8, 0xE7, 0x00, 0x00, 0x00]); // mov eax, 231 (exit_group)
    code.extend_from_slice(&[0x31, 0xFF]); // xor edi, edi
    code.extend_from_slice(&[0x0F, 0x05]); // syscall
    code.extend_from_slice(&[0xEB, 0xFE]); // jmp $ (filet de securite)

    // Le message est place juste apres le code ; `lea` est relatif a la fin de
    // l'instruction (rip pointe alors sur l'octet suivant).
    let message_offset = code.len();
    let lea_end = lea_patch + 4;
    let displacement = (message_offset - lea_end) as u32;
    code[lea_patch..lea_patch + 4].copy_from_slice(&displacement.to_le_bytes());
    code.extend_from_slice(message);

    // En-tetes ELF64 : un seul segment PT_LOAD, lecture + execution.
    let filesz = (CODE_OFF + code.len()) as u64;
    let mut image = Vec::with_capacity(filesz as usize);
    image.extend_from_slice(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 0]); // e_ident
    image.extend_from_slice(&[0; 8]);
    image.extend_from_slice(&2u16.to_le_bytes()); // e_type = ET_EXEC
    image.extend_from_slice(&62u16.to_le_bytes()); // e_machine = x86-64
    image.extend_from_slice(&1u32.to_le_bytes()); // e_version
    image.extend_from_slice(&(base + CODE_OFF as u64).to_le_bytes()); // e_entry
    image.extend_from_slice(&(EHDR as u64).to_le_bytes()); // e_phoff
    image.extend_from_slice(&0u64.to_le_bytes()); // e_shoff
    image.extend_from_slice(&0u32.to_le_bytes()); // e_flags
    image.extend_from_slice(&(EHDR as u16).to_le_bytes()); // e_ehsize
    image.extend_from_slice(&(PHDR as u16).to_le_bytes()); // e_phentsize
    image.extend_from_slice(&1u16.to_le_bytes()); // e_phnum
    image.extend_from_slice(&0u16.to_le_bytes()); // e_shentsize
    image.extend_from_slice(&0u16.to_le_bytes()); // e_shnum
    image.extend_from_slice(&0u16.to_le_bytes()); // e_shstrndx

    image.extend_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
    image.extend_from_slice(&5u32.to_le_bytes()); // p_flags = R|X
    image.extend_from_slice(&0u64.to_le_bytes()); // p_offset
    image.extend_from_slice(&base.to_le_bytes()); // p_vaddr
    image.extend_from_slice(&base.to_le_bytes()); // p_paddr
    image.extend_from_slice(&filesz.to_le_bytes()); // p_filesz
    image.extend_from_slice(&filesz.to_le_bytes()); // p_memsz
    image.extend_from_slice(&0x1000u64.to_le_bytes()); // p_align

    image.extend_from_slice(&code);
    image
}

/// Autotest complet du mode utilisateur (commande `usermode`).
pub fn selftest(cwd: usize) {
    crate::println!("Autotest ring 3 / ELF / syscalls");
    crate::println!("  GDT      : {}", crate::arch::x86_64::gdt::state());
    crate::println!("  user-mode: {}", crate::arch::x86_64::usermode::state());
    let (_, free, total) = vmm::frame_stats();
    crate::println!("  frames   : {} libres sur {}", free, total);

    let image = selftest_image();
    crate::println!("  image    : ELF64 genere en memoire, {} octets", image.len());

    let before = crate::kernel::abi::syscall_count();
    let argv = alloc::vec!["usermode-selftest".to_string()];
    match exec_image("usermode-selftest", &image, &argv, &default_environment(), cwd) {
        Ok(code) => {
            let used = crate::kernel::abi::syscall_count() - before;
            crate::println!("  resultat : sortie avec le code {} ({} appels systeme)", code, used);
            if code == 0 {
                crate::println!("  OK : ring 3, syscall/sysret, mmap et exit_group fonctionnels");
            }
        }
        Err(message) => crate::println!("  ECHEC : {}", message),
    }
}
