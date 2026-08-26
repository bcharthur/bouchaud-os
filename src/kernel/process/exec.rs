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
        alloc::format!(
            "QT_QPA_PLATFORM_PLUGIN_ARGS=fb=/dev/fb0:size={}x{}",
            width,
            height
        ),
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

/// Resout un executable sans recopier son contenu.
fn resolve_file_node(path: &str, cwd: usize) -> Result<usize, String> {
    let fs = crate::fs::ramfs::fs();
    match fs.resolve(path, cwd) {
        Some(node) if fs.nodes[node].kind == crate::fs::ramfs::NodeKind::File => Ok(node),
        Some(_) => Err(alloc::format!("{} : est un repertoire", path)),
        None => Err(alloc::format!("{} : fichier introuvable", path)),
    }
}

#[derive(Clone, Copy)]
enum ImageSource<'a> {
    Node(usize),
    Memory(&'a [u8]),
}

/// Charge et execute un programme ELF, et attend sa fin.
///
/// Renvoie le code de sortie du processus.
pub fn exec(path: &str, argv: &[String], envp: &[String], cwd: usize) -> Result<i32, String> {
    if !crate::arch::x86_64::usermode::ready() {
        return Err("user-mode non initialise (ring 3 indisponible)".to_string());
    }
    let node = resolve_file_node(path, cwd)?;
    exec_source(path, ImageSource::Node(node), argv, envp, cwd)
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
    prepare: &mut dyn FnMut(&task::Process) -> Vec<String>,
) -> Result<u32, String> {
    if !crate::arch::x86_64::usermode::ready() {
        return Err("user-mode non initialise (ring 3 indisponible)".to_string());
    }
    let node = resolve_file_node(path, cwd)?;
    let (process, task) = construit_tache(
        path,
        ImageSource::Node(node),
        argv,
        envp,
        cwd,
        Some(prepare),
    )?;
    let pid = process.pid;
    task::register(task);
    Ok(pid)
}

/// Meme chose a partir d'une image ELF deja en memoire (autotest embarque).
/// Refuse une image que ce noyau ne sait pas executer, en NOMMANT ce qu'elle est.
///
/// `elf::parse` repondait « signature ELF absente » a un `.exe`, ce qui est
/// vrai et n'apprend rien. Un `.exe` compile pour Windows echouera de toute
/// facon ; la difference est qu'il le dira, et pourquoi.
fn verifie_format(name: &str, data: &[u8]) -> Result<(), String> {
    use crate::kernel::loader::{identifie, pe, Format};

    match identifie(data) {
        Format::Elf64 => Ok(()),
        Format::Pe32Plus => match pe::lit_entete(data) {
            Err(refus) => Err(alloc::format!(
                "{} : PE32+ illisible ({:?})",
                name, refus
            )),
            Ok(_) => Err(alloc::format!(
                "{} : PE32+ reconnu, mais ce noyau ne sait pas encore le charger",
                name
            )),
        },
        Format::Script => Err(alloc::format!(
            "{} : script #! -- l'interpreteur doit etre lance explicitement",
            name
        )),
        Format::Inconnu => Err(alloc::format!(
            "{} : format non reconnu (ni ELF64, ni PE32+, ni script)",
            name
        )),
    }
}

pub fn exec_image(
    name: &str,
    data: &[u8],
    argv: &[String],
    envp: &[String],
    cwd: usize,
) -> Result<i32, String> {
    exec_source(name, ImageSource::Memory(data), argv, envp, cwd)
}

fn exec_source(
    name: &str,
    source: ImageSource<'_>,
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
    let (process, first) = construit_tache(name, source, argv, envp, cwd, None)?;
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
    source: ImageSource<'_>,
    argv: &[String],
    envp: &[String],
    cwd: usize,
    prepare: Option<&mut dyn FnMut(&task::Process) -> Vec<String>>,
) -> Result<
    (
        alloc::sync::Arc<task::Process>,
        alloc::boxed::Box<task::Task>,
    ),
    String,
> {
    crate::kernel::perf::exec_start(name);
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
            for entree in prepare(&process) {
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
        let metadata = process.metadata.lock();
        let uid = metadata.uid;
        let gid = metadata.gid;
        drop(metadata);
        let mut borrowed = process.mm.lock();

        let image = match source {
            ImageSource::Memory(data) => {
                verifie_format(name, data)?;
                elf::load(&mut borrowed.space, data, vmm::user_load_base())
            }
            ImageSource::Node(node) => {
                elf::load_node_lazy(node, vmm::user_load_base(), &mut borrowed.promesses)
            }
        }
        .map_err(|message| alloc::format!("{} : {}", name, message))?;

        // Binaire dynamique : on charge aussi l'editeur de liens et on lui donne
        // la main. Le programme est deja en memoire, ld.so le trouvera via
        // AT_PHDR/AT_ENTRY.
        let (entry, interp_base) = match image.interp.as_deref() {
            None => (image.entry, 0),
            Some(interp_path) => {
                let interp_node = resolve_file_node(interp_path, cwd).map_err(|_| {
                    alloc::format!(
                        "{} : interpreteur {} absent du namespace",
                        name,
                        interp_path
                    )
                })?;
                let interp = elf::load_node_lazy(
                    interp_node,
                    vmm::user_interp_base(),
                    &mut borrowed.promesses,
                )
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
            uid,
            gid,
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

/// Petit ELF64 autonome, genere en memoire, qui reproduit le reveil utilise par
/// Ladybird LibIPC sans aucune libc ni outil Linux hote.
///
/// Deux threads du meme processus partagent un `pipe2`. Le parent entre dans
/// `poll(..., -1)` ; le second thread dort 50 ms puis ecrit un octet. Le succes
/// prouve le chemin exact qui bloquait `TransportSocket::wake_io_thread()` :
///
///     clone(CLONE_THREAD) -> pipe2 -> poll(-1) <- write depuis l'autre thread
///
/// L'image n'est jamais stockee dans le userland : elle est construite par le
/// noyau a l'execution de `poll-selftest`.
pub fn poll_selftest_image() -> Vec<u8> {
    let base: u64 = vmm::user_load_base();
    const EHDR: usize = 64;
    const PHDR: usize = 56;
    const CODE_OFF: usize = EHDR + PHDR;

    let succes = b"[poll-selftest] OK : poll reveille par write(pipe) depuis un autre thread\n";
    let echec = b"[poll-selftest] ECHEC : le reveil inter-thread n'a pas traverse poll\n";

    let mut code: Vec<u8> = Vec::new();
    let mut vers_echec: Vec<usize> = Vec::new();

    // mmap(NULL, 4096, RW, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)
    // La page porte pipefd[2], pollfd, timespec et les octets de test.
    code.extend_from_slice(&[0xB8, 0x09, 0x00, 0x00, 0x00]); // mov eax, 9
    code.extend_from_slice(&[0x31, 0xFF]); // xor edi, edi
    code.extend_from_slice(&[0xBE, 0x00, 0x10, 0x00, 0x00]); // mov esi, 4096
    code.extend_from_slice(&[0xBA, 0x03, 0x00, 0x00, 0x00]); // mov edx, PROT_READ|PROT_WRITE
    code.extend_from_slice(&[0x41, 0xBA, 0x22, 0x00, 0x00, 0x00]); // mov r10d, MAP_PRIVATE|MAP_ANON
    code.extend_from_slice(&[0x49, 0xC7, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]); // mov r8, -1
    code.extend_from_slice(&[0x45, 0x31, 0xC9]); // xor r9d, r9d
    code.extend_from_slice(&[0x0F, 0x05]); // syscall
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
    code.extend_from_slice(&[0x0F, 0x88, 0, 0, 0, 0]); // js fail
    vers_echec.push(code.len() - 4);
    code.extend_from_slice(&[0x49, 0x89, 0xC4]); // mov r12, rax

    // pipe2(state, O_NONBLOCK|O_CLOEXEC)
    code.extend_from_slice(&[0xB8, 0x25, 0x01, 0x00, 0x00]); // mov eax, 293
    code.extend_from_slice(&[0x4C, 0x89, 0xE7]); // mov rdi, r12
    code.extend_from_slice(&[0xBE, 0x00, 0x08, 0x08, 0x00]); // mov esi, 0x80800
    code.extend_from_slice(&[0x0F, 0x05]);
    code.extend_from_slice(&[0x48, 0x85, 0xC0]);
    code.extend_from_slice(&[0x0F, 0x88, 0, 0, 0, 0]); // js fail
    vers_echec.push(code.len() - 4);

    // mmap(NULL, 64 KiB, RW, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0) pour la pile enfant.
    code.extend_from_slice(&[0xB8, 0x09, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x31, 0xFF]);
    code.extend_from_slice(&[0xBE, 0x00, 0x00, 0x01, 0x00]); // 65536
    code.extend_from_slice(&[0xBA, 0x03, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x41, 0xBA, 0x22, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x49, 0xC7, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]);
    code.extend_from_slice(&[0x45, 0x31, 0xC9]);
    code.extend_from_slice(&[0x0F, 0x05]);
    code.extend_from_slice(&[0x48, 0x85, 0xC0]);
    code.extend_from_slice(&[0x0F, 0x88, 0, 0, 0, 0]); // js fail
    vers_echec.push(code.len() - 4);
    code.extend_from_slice(&[0x49, 0x89, 0xC5]); // mov r13, rax

    // timespec { 0 s, 50 000 000 ns } a state+32.
    code.extend_from_slice(&[0x49, 0xC7, 0x44, 0x24, 0x20, 0, 0, 0, 0]);
    code.extend_from_slice(&[0x49, 0xC7, 0x44, 0x24, 0x28, 0x80, 0xF0, 0xFA, 0x02]);

    // clone(CLONE_VM|CLONE_FS|CLONE_FILES|CLONE_SIGHAND|CLONE_THREAD,
    //       stack_top, 0, 0, 0)
    code.extend_from_slice(&[0xB8, 0x38, 0x00, 0x00, 0x00]); // mov eax, 56
    code.extend_from_slice(&[0xBF, 0x00, 0x0F, 0x01, 0x00]); // mov edi, 0x10f00
    code.extend_from_slice(&[0x49, 0x8D, 0xB5, 0xF0, 0xFF, 0x00, 0x00]); // lea rsi,[r13+0xfff0]
    code.extend_from_slice(&[0x31, 0xD2]); // xor edx, edx
    code.extend_from_slice(&[0x45, 0x31, 0xD2]); // xor r10d, r10d
    code.extend_from_slice(&[0x45, 0x31, 0xC0]); // xor r8d, r8d
    code.extend_from_slice(&[0x0F, 0x05]);
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax,rax
    code.extend_from_slice(&[0x0F, 0x88, 0, 0, 0, 0]); // js fail
    vers_echec.push(code.len() - 4);
    code.extend_from_slice(&[0x0F, 0x84, 0, 0, 0, 0]); // jz child
    let vers_enfant = code.len() - 4;

    // Parent : pollfd { fd=pipe[0], events=POLLIN, revents=0 } a state+16.
    code.extend_from_slice(&[0x41, 0x8B, 0x3C, 0x24]); // mov edi,[r12]
    code.extend_from_slice(&[0x41, 0x89, 0x7C, 0x24, 0x10]); // mov [r12+16],edi
    code.extend_from_slice(&[0x66, 0x41, 0xC7, 0x44, 0x24, 0x14, 0x01, 0x00]); // events=POLLIN
    code.extend_from_slice(&[0x66, 0x41, 0xC7, 0x44, 0x24, 0x16, 0x00, 0x00]); // revents=0

    // poll(&pollfd, 1, -1)
    code.extend_from_slice(&[0xB8, 0x07, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x49, 0x8D, 0x7C, 0x24, 0x10]);
    code.extend_from_slice(&[0xBE, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xBA, 0xFF, 0xFF, 0xFF, 0xFF]);
    code.extend_from_slice(&[0x0F, 0x05]);
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x01]); // cmp rax,1
    code.extend_from_slice(&[0x0F, 0x85, 0, 0, 0, 0]); // jne fail
    vers_echec.push(code.len() - 4);

    // revents & POLLIN
    code.extend_from_slice(&[0x41, 0x0F, 0xB7, 0x44, 0x24, 0x16]); // movzx eax,word [r12+22]
    code.extend_from_slice(&[0xA9, 0x01, 0x00, 0x00, 0x00]); // test eax,1
    code.extend_from_slice(&[0x0F, 0x84, 0, 0, 0, 0]); // jz fail
    vers_echec.push(code.len() - 4);

    // read(pipe[0], state+65, 1)
    code.extend_from_slice(&[0x41, 0x8B, 0x3C, 0x24]);
    code.extend_from_slice(&[0x49, 0x8D, 0x74, 0x24, 0x41]);
    code.extend_from_slice(&[0xBA, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x31, 0xC0]);
    code.extend_from_slice(&[0x0F, 0x05]);
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x01]);
    code.extend_from_slice(&[0x0F, 0x85, 0, 0, 0, 0]); // jne fail
    vers_echec.push(code.len() - 4);
    code.extend_from_slice(&[0x41, 0x80, 0x7C, 0x24, 0x41, 0xA5]); // cmp byte [r12+65],0xa5
    code.extend_from_slice(&[0x0F, 0x85, 0, 0, 0, 0]); // jne fail
    vers_echec.push(code.len() - 4);

    // write(stdout, success, len)
    code.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0x8D, 0x35, 0, 0, 0, 0]);
    let succes_lea = code.len() - 4;
    code.extend_from_slice(&[0xBA]);
    code.extend_from_slice(&(succes.len() as u32).to_le_bytes());
    code.extend_from_slice(&[0x0F, 0x05]);
    code.extend_from_slice(&[0xB8, 0xE7, 0x00, 0x00, 0x00]); // exit_group(0)
    code.extend_from_slice(&[0x31, 0xFF]);
    code.extend_from_slice(&[0x0F, 0x05]);

    // Enfant : attendre assez pour que le parent soit deja dans poll(-1).
    let enfant = code.len();
    code.extend_from_slice(&[0xB8, 0x23, 0x00, 0x00, 0x00]); // nanosleep
    code.extend_from_slice(&[0x49, 0x8D, 0x7C, 0x24, 0x20]);
    code.extend_from_slice(&[0x31, 0xF6]);
    code.extend_from_slice(&[0x0F, 0x05]);
    // state+64 = 0xa5 ; write(pipe[1], state+64, 1)
    code.extend_from_slice(&[0x41, 0xC6, 0x44, 0x24, 0x40, 0xA5]);
    code.extend_from_slice(&[0x41, 0x8B, 0x7C, 0x24, 0x04]);
    code.extend_from_slice(&[0x49, 0x8D, 0x74, 0x24, 0x40]);
    code.extend_from_slice(&[0xBA, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x0F, 0x05]);
    // exit(0) : termine uniquement ce thread.
    code.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x31, 0xFF]);
    code.extend_from_slice(&[0x0F, 0x05]);
    code.extend_from_slice(&[0xEB, 0xFE]);

    // Echec commun.
    let fail = code.len();
    code.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x48, 0x8D, 0x35, 0, 0, 0, 0]);
    let echec_lea = code.len() - 4;
    code.extend_from_slice(&[0xBA]);
    code.extend_from_slice(&(echec.len() as u32).to_le_bytes());
    code.extend_from_slice(&[0x0F, 0x05]);
    code.extend_from_slice(&[0xB8, 0xE7, 0x00, 0x00, 0x00]); // exit_group(1)
    code.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&[0x0F, 0x05]);
    code.extend_from_slice(&[0xEB, 0xFE]);

    let succes_offset = code.len();
    code.extend_from_slice(succes);
    let echec_offset = code.len();
    code.extend_from_slice(echec);

    // Patcher les branches rel32 et les LEA RIP-relatifs.
    let patch_rel32 = |code: &mut Vec<u8>, at: usize, target: usize| {
        let disp = target as i64 - (at + 4) as i64;
        code[at..at + 4].copy_from_slice(&(disp as i32).to_le_bytes());
    };
    patch_rel32(&mut code, vers_enfant, enfant);
    for at in vers_echec {
        patch_rel32(&mut code, at, fail);
    }
    patch_rel32(&mut code, succes_lea, succes_offset);
    patch_rel32(&mut code, echec_lea, echec_offset);

    let filesz = (CODE_OFF + code.len()) as u64;
    let mut image = Vec::with_capacity(filesz as usize);
    image.extend_from_slice(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 0]);
    image.extend_from_slice(&[0; 8]);
    image.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    image.extend_from_slice(&62u16.to_le_bytes()); // x86-64
    image.extend_from_slice(&1u32.to_le_bytes());
    image.extend_from_slice(&(base + CODE_OFF as u64).to_le_bytes());
    image.extend_from_slice(&(EHDR as u64).to_le_bytes());
    image.extend_from_slice(&0u64.to_le_bytes());
    image.extend_from_slice(&0u32.to_le_bytes());
    image.extend_from_slice(&(EHDR as u16).to_le_bytes());
    image.extend_from_slice(&(PHDR as u16).to_le_bytes());
    image.extend_from_slice(&1u16.to_le_bytes());
    image.extend_from_slice(&0u16.to_le_bytes());
    image.extend_from_slice(&0u16.to_le_bytes());
    image.extend_from_slice(&0u16.to_le_bytes());

    image.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    image.extend_from_slice(&5u32.to_le_bytes()); // R|X
    image.extend_from_slice(&0u64.to_le_bytes());
    image.extend_from_slice(&base.to_le_bytes());
    image.extend_from_slice(&base.to_le_bytes());
    image.extend_from_slice(&filesz.to_le_bytes());
    image.extend_from_slice(&filesz.to_le_bytes());
    image.extend_from_slice(&0x1000u64.to_le_bytes());
    image.extend_from_slice(&code);
    image
}

/// Commande bare-metal `poll-selftest`.
///
/// Aucun binaire userland preconstruit, aucune libc, aucun WSL : le noyau
/// fabrique un ELF x86-64 minimal en RAM et l'execute en ring 3.
pub fn poll_selftest(cwd: usize) -> i32 {
    crate::println!("Autotest Ladybird M5 : poll/pipe/reveil inter-thread");
    crate::println!("  mode     : ELF genere par Bouchaud OS, sans dependance hote");
    crate::println!("  scenario : parent poll(-1), enfant clone dort 50 ms puis write(pipe)");

    let image = poll_selftest_image();
    let argv = alloc::vec!["poll-selftest".to_string()];
    let before = crate::kernel::abi::syscall_count();
    match exec_image("poll-selftest", &image, &argv, &default_environment(), cwd) {
        Ok(code) => {
            let used = crate::kernel::abi::syscall_count().saturating_sub(before);
            crate::println!("  sortie   : code {} ({} appels systeme)", code, used);
            if code == 0 {
                crate::println!("  RESULTAT : M5 wakeup poll/pipe OK");
                0
            } else {
                crate::println!("  RESULTAT : ECHEC M5 wakeup poll/pipe");
                code
            }
        }
        Err(message) => {
            crate::println!("  ECHEC : {}", message);
            crate::println!(
                "  note  : depuis le bureau, relancer ce test dans le shell texte avant `desktop`"
            );
            1
        }
    }
}

/// Autotest complet du mode utilisateur (commande `usermode`).
pub fn selftest(cwd: usize) {
    crate::println!("Autotest ring 3 / ELF / syscalls");
    crate::println!("  GDT      : {}", crate::arch::x86_64::gdt::state());
    crate::println!("  user-mode: {}", crate::arch::x86_64::usermode::state());
    let (_, free, total) = vmm::frame_stats();
    crate::println!("  frames   : {} libres sur {}", free, total);

    let image = selftest_image();
    crate::println!(
        "  image    : ELF64 genere en memoire, {} octets",
        image.len()
    );

    let before = crate::kernel::abi::syscall_count();
    let argv = alloc::vec!["usermode-selftest".to_string()];
    match exec_image(
        "usermode-selftest",
        &image,
        &argv,
        &default_environment(),
        cwd,
    ) {
        Ok(code) => {
            let used = crate::kernel::abi::syscall_count() - before;
            crate::println!(
                "  resultat : sortie avec le code {} ({} appels systeme)",
                code,
                used
            );
            if code == 0 {
                crate::println!("  OK : ring 3, syscall/sysret, mmap et exit_group fonctionnels");
            }
        }
        Err(message) => crate::println!("  ECHEC : {}", message),
    }
}
