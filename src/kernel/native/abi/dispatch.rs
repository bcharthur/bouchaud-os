use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::arch::x86_64::usermode::TrapFrame;

use super::numbers;
use super::types::{
    Error, HandleId, ObjectKind, Result, Rights, ABI_VERSION_PACKED,
};
use super::wire::{HandleInfo, ProcessInfo, RecvMeta, SpawnResult, UpdateState, WaitEvent,
                  PROCESS_RUNNING, PROCESS_TERMINATED};
use crate::kernel::native::event::Event;
use crate::kernel::native::handle::{self, Entry};
use crate::kernel::native::ipc::{
    ChannelEndpoint, Message, TransferredHandle, MAX_MESSAGE_BYTES, MAX_MESSAGE_HANDLES,
};
use crate::kernel::native::object::Object;
use crate::kernel::native::shm::SharedRegion;
use crate::kernel::native::waitset::WaitSet;

fn result_u64(result: Result<u64>) -> i64 {
    match result {
        Ok(value) if value <= i64::MAX as u64 => value as i64,
        Ok(_) => Error::InvalidArgument.neg(),
        Err(error) => error.neg(),
    }
}

fn result_unit(result: Result<()>) -> i64 {
    match result { Ok(()) => 0, Err(error) => error.neg() }
}

fn pair_to_user(out: u64) -> Result<u64> {
    if out == 0 { return Err(Error::Fault); }
    let (left, right) = ChannelEndpoint::pair();
    let table = handle::current_table();

    let left_id = table.insert(Arc::new(Object::Channel(left)), Rights::CHANNEL_DEFAULT)?;
    let right_id = match table.insert(Arc::new(Object::Channel(right)), Rights::CHANNEL_DEFAULT) {
        Ok(id) => id,
        Err(error) => {
            let _ = table.close(left_id);
            return Err(error);
        }
    };

    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&left_id.raw().to_le_bytes());
    bytes[8..].copy_from_slice(&right_id.raw().to_le_bytes());
    if let Err(error) = super::usercopy::write(out, &bytes) {
        let _ = table.close(left_id);
        let _ = table.close(right_id);
        return Err(error);
    }
    Ok(0)
}

/// Envoi, avec ou sans attenuation des droits transferes.
///
/// `masques` est vide pour l'envoi ordinaire : chaque handle franchit alors la
/// frontiere avec `Rights::TOUS` comme plafond, c'est-a-dire tel quel. Sinon il
/// porte exactement un masque par handle, et chaque capacite arrive attenuee a
/// l'intersection de ses droits et de son masque.
fn channel_send_avec_masques(args: [u64; 6], masques_ptr: u64) -> Result<u64> {
    let handle_id = HandleId::from_raw(args[0]);
    let data_len = args[2] as usize;
    let handle_count = args[4] as usize;

    if data_len > MAX_MESSAGE_BYTES || handle_count > MAX_MESSAGE_HANDLES {
        return Err(Error::TooLarge);
    }

    let bytes = super::usercopy::read(args[1], data_len)?;
    let ids = super::usercopy::read_handles(args[3], handle_count)?;
    // Les masques sont lus AVANT toute mutation : un tableau illisible doit
    // faire echouer l'envoi sans avoir rien transfere.
    let masques = if masques_ptr == 0 {
        Vec::new()
    } else {
        let brut = super::usercopy::read(masques_ptr, handle_count * 4)?;
        brut.chunks_exact(4)
            .map(|mot| Rights(u32::from_le_bytes([mot[0], mot[1], mot[2], mot[3]])))
            .collect()
    };
    if !masques.is_empty() && masques.len() != ids.len() {
        return Err(Error::InvalidArgument);
    }

    let table = handle::current_table();
    let endpoint = table.lookup_kind(handle_id, Rights::WRITE, ObjectKind::Channel)?;

    let mut transferred = Vec::with_capacity(ids.len());
    for (index, id) in ids.iter().enumerate() {
        let masque = masques.get(index).copied().unwrap_or(Rights::TOUS);
        let entry = table.export(*id, masque)?;
        // BOUCHAUD_NATIVE_CANAL_SANS_CYCLE_V1 : ni l'extremite emettrice, ni
        // son pair, ne traversent leur propre canal (cycle de references).
        if let (Object::Channel(sender), Object::Channel(moved)) =
            (endpoint.object.as_ref(), entry.object.as_ref())
        {
            if sender.same_channel(moved) {
                return Err(Error::InvalidArgument);
            }
        }
        transferred.push(TransferredHandle { object: entry.object, rights: entry.rights });
    }

    match endpoint.object.as_ref() {
        Object::Channel(channel) => channel.send(Message::new(bytes, transferred)).map(|n| n as u64),
        _ => Err(Error::WrongType),
    }
}

fn channel_send(args: [u64; 6]) -> Result<u64> {
    channel_send_avec_masques(args, 0)
}

fn channel_recv(args: [u64; 6]) -> Result<u64> {
    let handle_id = HandleId::from_raw(args[0]);
    let data_cap = args[2] as usize;
    let handles_cap = args[4] as usize;
    let meta_ptr = args[5];

    let table = handle::current_table();
    let endpoint = table.lookup_kind(handle_id, Rights::READ, ObjectKind::Channel)?;

    let channel = match endpoint.object.as_ref() {
        Object::Channel(channel) => channel,
        _ => return Err(Error::WrongType),
    };

    let (needed_bytes, needed_handles) = channel.front_sizes()?;
    if needed_bytes > data_cap || needed_handles > handles_cap {
        if meta_ptr != 0 {
            let meta = RecvMeta { bytes: needed_bytes as u64, handles: needed_handles as u64 };
            super::usercopy::write(meta_ptr, &meta.bytes_le())?;
        }
        return Err(Error::BufferTooSmall);
    }

    let message = channel.recv()?;
    // ARCH_P1_IPC_RECV_ATOMIC: front_sizes() and recv() are distinct lock
    // acquisitions. Another receiver can change the front in between.
    // Never copy a message larger than the actual user-supplied capacities.
    if message.bytes.len() > data_cap || message.handles.len() > handles_cap {
        let meta = RecvMeta { bytes: message.bytes.len() as u64,
                              handles: message.handles.len() as u64 };
        // Always restore the original message, including on invalid meta ptr.
        channel.requeue_front(message);
        if meta_ptr != 0 {
            super::usercopy::write(meta_ptr, &meta.bytes_le())?;
        }
        return Err(Error::BufferTooSmall);
    }
    // BOUCHAUD_MATURITY_IPC_PREFLIGHT_V1: preflight copyout BEFORE installing handles.
    // This avoids deterministically invalid outputs leaving transient handles.
    // Memory mappings can still change: existing rollback below is essential.
    let copyout_preflight = (|| -> Result<()> {
        super::usercopy::validate_write(args[1], message.bytes.len())?;
        super::usercopy::validate_write(
            args[3], message.handles.len().checked_mul(8).ok_or(Error::TooLarge)?,
        )?;
        if meta_ptr != 0 {
            super::usercopy::validate_write(meta_ptr, RecvMeta::BYTE_LEN)?;
        }
        Ok(())
    })();
    if let Err(error) = copyout_preflight {
        channel.requeue_front(message);
        return Err(error);
    }
    let entries: Vec<Entry> = message.handles.iter().map(|item| Entry {
        object: Arc::clone(&item.object),
        rights: item.rights,
    }).collect();

    let installed = match table.insert_many(&entries) {
        Ok(installed) => installed,
        Err(error) => {
            channel.requeue_front(message);
            return Err(error);
        }
    };

    // User copies happen after the kernel-side transaction. If copyout fails,
    // close any just-installed handles so they cannot leak.
    if let Err(error) = super::usercopy::write(args[1], &message.bytes) {
        for id in installed { let _ = table.close(id); }
        channel.requeue_front(message);
        return Err(error);
    }
    if let Err(error) = super::usercopy::write_handles(args[3], &installed) {
        for id in installed { let _ = table.close(id); }
        channel.requeue_front(message);
        return Err(error);
    }
    if meta_ptr != 0 {
        let meta = RecvMeta { bytes: message.bytes.len() as u64, handles: installed.len() as u64 };
        if let Err(error) = super::usercopy::write(meta_ptr, &meta.bytes_le()) {
            for id in installed { let _ = table.close(id); }
            channel.requeue_front(message);
            return Err(error);
        }
    }

    Ok(message.bytes.len() as u64)
}

fn handle_info(args: [u64; 6]) -> Result<u64> {
    let table = handle::current_table();
    let entry = table.lookup(HandleId::from_raw(args[0]), Rights::INSPECT)?;
    let info = HandleInfo::new(entry.object.kind(), entry.rights, entry.object.signals());
    super::usercopy::write(args[1], &info.bytes_le())?;
    Ok(0)
}

fn event_query(args: [u64; 6]) -> Result<u64> {
    let table = handle::current_table();
    let entry = table.lookup_kind(HandleId::from_raw(args[0]), Rights::WAIT, ObjectKind::Event)?;
    match entry.object.as_ref() {
        Object::Event(event) => Ok(((event.sequence() & 0x7fff_ffff_ffff_ffff) << 1)
            | u64::from(event.is_signaled())),
        _ => Err(Error::WrongType),
    }
}

fn waitset_poll(args: [u64; 6]) -> Result<u64> {
    let cap = args[2] as usize;
    if cap > 4096 { return Err(Error::TooLarge); }
    let table = handle::current_table();
    let entry = table.lookup_kind(
        HandleId::from_raw(args[0]), Rights::READ, ObjectKind::WaitSet
    )?;
    let ready = match entry.object.as_ref() {
        Object::WaitSet(waitset) => waitset.poll(cap),
        _ => return Err(Error::WrongType),
    };

    let mut bytes = Vec::with_capacity(ready.len() * WaitEvent::BYTE_LEN);
    for (key, signals) in ready {
        bytes.extend_from_slice(&WaitEvent { key, signals: signals.0, reserved: 0 }.bytes_le());
    }
    super::usercopy::write(args[1], &bytes)?;
    Ok((bytes.len() / WaitEvent::BYTE_LEN) as u64)
}

/// Ecrit les evenements prets ; rend leur nombre.
fn waitset_write(out: u64, ready: Vec<(u64, super::types::Signals)>) -> Result<u64> {
    let mut bytes = Vec::with_capacity(ready.len() * WaitEvent::BYTE_LEN);
    for (key, signals) in ready {
        bytes.extend_from_slice(&WaitEvent { key, signals: signals.0, reserved: 0 }.bytes_le());
    }
    super::usercopy::write(out, &bytes)?;
    Ok((bytes.len() / WaitEvent::BYTE_LEN) as u64)
}

/// BOUCHAUD_NATIVE_WAITSET_WAIT_V1 -- attente bloquante, level-triggered.
///
/// args: waitset, out, cap, timeout_ns.
///
/// Semantique : rend >0 des qu'au moins un objet surveille porte un signal ;
/// `TimedOut` a l'echeance ; 0 si un signal POSIX destine a ce fil a
/// interrompu l'attente (l'appelant reboucle). `timeout_ns == 0` equivaut a
/// `WAITSET_POLL`.
///
/// Reveil (BOUCHAUD_NATIVE_ACTIVITE_V1) : le ticket de la source d'activite
/// native est pris AVANT le poll. Toute montee de signal posterieure fait
/// avancer sa generation : l'attente rend aussitot au lieu de dormir. Aucun
/// verrou n'est tenu pendant l'attente. La tranche `TRANCHE_NS` ne sert plus
/// qu'a relire les signaux POSIX et l'etat zombie ; elle n'intervient pas
/// dans la latence d'un evenement natif.
fn waitset_wait(args: [u64; 6]) -> Result<u64> {
    const TRANCHE_NS: u64 = 20_000_000;
    let cap = args[2] as usize;
    if cap == 0 { return Err(Error::InvalidArgument); }
    if cap > 4096 { return Err(Error::TooLarge); }
    super::usercopy::validate_write(
        args[1], cap.checked_mul(WaitEvent::BYTE_LEN).ok_or(Error::TooLarge)?,
    )?;
    let entry = handle::current_table().lookup_kind(
        HandleId::from_raw(args[0]), Rights::READ, ObjectKind::WaitSet
    )?;
    let waitset = match entry.object.as_ref() {
        Object::WaitSet(waitset) => waitset,
        _ => return Err(Error::WrongType),
    };
    let timeout = args[3];
    let deadline = crate::kernel::timer::monotonic_ns().saturating_add(timeout);
    loop {
        let ticket = crate::kernel::native::activite::ticket();
        let ready = waitset.poll(cap);
        if !ready.is_empty() {
            return waitset_write(args[1], ready);
        }
        if timeout == 0 {
            return Ok(0);
        }
        let now = crate::kernel::timer::monotonic_ns();
        if now >= deadline {
            return Err(Error::TimedOut);
        }
        let echeance = deadline.min(now.saturating_add(TRANCHE_NS));
        let reveil = crate::kernel::native::activite::attend(ticket, echeance);
        // BOUCHAUD_IPC_ATTRIBUTION_V1 : mesure seule.
        crate::kernel::native::activite::note_reprise(
            matches!(reveil, crate::kernel::sync::WaitSourceWake::Signaled));
        if crate::kernel::task::signal_interrompt_attente() {
            return Ok(0);
        }
        if crate::kernel::task::try_current()
            .map(|t| t.state == crate::kernel::task::TaskState::Zombie)
            .unwrap_or(false)
        {
            return Ok(0);
        }
    }
}

/// BOUCHAUD_NATIVE_PROCESS_SPAWN_V1
///
/// args: path, path_len, handles, count, masks (0 = sans attenuation), out.
///
/// Tout est lu et valide AVANT le lancement : un argument illisible ne
/// laisse ni processus, ni handle. Les handles transferes exigent TRANSFER
/// (comme un envoi sur canal) et sont attenues au masque. Le fils est un vrai
/// fils (filiation, uid/gid de l'appelant, politique heritee avant d'etre
/// ordonnancable : exec::lance_enfant).
fn process_spawn(args: [u64; 6]) -> Result<u64> {
    const MAX_PATH: usize = 255;
    const MAX_SPAWN_HANDLES: usize = 8;
    let path_len = args[1] as usize;
    if path_len == 0 || path_len > MAX_PATH { return Err(Error::InvalidArgument); }
    let count = args[3] as usize;
    if count > MAX_SPAWN_HANDLES { return Err(Error::TooLarge); }
    let out = args[5];
    super::usercopy::validate_write(out, SpawnResult::BYTE_LEN)?;

    let raw = super::usercopy::read(args[0], path_len)?;
    let path = core::str::from_utf8(&raw).map_err(|_| Error::InvalidArgument)?;
    if !path.starts_with('/') || path.contains('\0') { return Err(Error::InvalidArgument); }
    let path = alloc::string::String::from(path);

    let ids = super::usercopy::read_handles(args[2], count)?;
    let masques: Vec<Rights> = if args[4] == 0 {
        alloc::vec![Rights::TOUS; count]
    } else {
        super::usercopy::read(args[4], count * 4)?
            .chunks_exact(4)
            .map(|m| Rights(u32::from_le_bytes([m[0], m[1], m[2], m[3]])))
            .collect()
    };

    let table = handle::current_table();
    let mut entries: Vec<Entry> = Vec::with_capacity(count);
    for (id, masque) in ids.iter().zip(masques.iter()) {
        entries.push(table.export(*id, *masque)?);
    }

    let appelant = crate::kernel::task::current_process();
    let (cwd, uid, gid) = {
        let meta = appelant.metadata.lock();
        (meta.cwd, meta.uid, meta.gid)
    };
    let filiation = crate::kernel::exec::Filiation { parent: appelant.pid, uid, gid };

    let mut installes: Option<Vec<HandleId>> = None;
    let argv = alloc::vec![path.clone()];
    let env = alloc::vec![
        alloc::string::String::from("PATH=/bin:/usr/bin"),
    ];
    let lance = crate::kernel::exec::lance_enfant(
        &path, &argv, &env, cwd, filiation,
        &mut |process| {
            let child = handle::registry::table_for(process.pid);
            match child.insert_many(&entries) {
                Ok(ids) => {
                    let mut liste = alloc::string::String::from("BO_BOOTSTRAP=");
                    for (i, id) in ids.iter().enumerate() {
                        if i > 0 { liste.push(','); }
                        liste.push_str(&alloc::format!("{}", id.raw()));
                    }
                    installes = Some(ids);
                    alloc::vec![liste]
                }
                Err(_) => alloc::vec![],
            }
        },
    );
    let pid = match lance {
        Ok(pid) => pid,
        Err(raison) => {
            crate::kernel::dmesg::log_fmt(format_args!(
                "NATIVE_SPAWN_REFUS pid={} chemin={} raison={}",
                appelant.pid, path, raison));
            return Err(if raison.contains("introuvable") || raison.contains("repertoire") {
                Error::NotFound
            } else {
                Error::AccessDenied
            });
        }
    };
    if installes.map(|ids| ids.len()) != Some(entries.len()) {
        // Le fils tourne sans ses capacites : il n'a pas le contrat promis.
        crate::kernel::task::tue_processus(pid, 137);
        return Err(Error::NoSpace);
    }

    let suivi = crate::kernel::native::process::suit(pid);
    let process_handle = match table.insert(Arc::new(Object::Process(suivi)), Rights::PROCESS_DEFAULT) {
        Ok(id) => id,
        Err(error) => {
            crate::kernel::task::tue_processus(pid, 137);
            return Err(error);
        }
    };
    let result = SpawnResult { pid: pid as u64, process: process_handle.raw() };
    if let Err(error) = super::usercopy::write(out, &result.bytes_le()) {
        let _ = table.close(process_handle);
        crate::kernel::task::tue_processus(pid, 137);
        return Err(error);
    }
    crate::kernel::dmesg::log_fmt(format_args!(
        "NATIVE_SPAWN t={} parent={} pid={} chemin={} handles={}",
        crate::kernel::timer::monotonic_ms(), appelant.pid, pid, path, entries.len()));
    Ok(0)
}

fn process_info(args: [u64; 6]) -> Result<u64> {
    let entry = handle::current_table().lookup_kind(
        HandleId::from_raw(args[0]), Rights::INSPECT, ObjectKind::Process
    )?;
    let info = match entry.object.as_ref() {
        Object::Process(p) => ProcessInfo {
            pid: p.pid() as u64,
            state: if p.terminated() { PROCESS_TERMINATED } else { PROCESS_RUNNING },
            exit_code: if p.terminated() { p.exit_code() } else { 0 },
        },
        _ => return Err(Error::WrongType),
    };
    super::usercopy::write(args[1], &info.bytes_le())?;
    Ok(0)
}

// BOUCHAUD_UPDATE_MECANISME_V1 (ABI 1.2). Le droit est verifie AVANT toute
// entree-sortie : un handle sans le droit ne touche jamais le disque.
fn update_state(args: [u64; 6]) -> Result<u64> {
    handle::current_table().lookup_kind(HandleId::from_raw(args[0]), Rights::INSPECT, ObjectKind::Update)?;
    super::usercopy::validate_write(args[1], UpdateState::BYTE_LEN)?;
    let e = crate::kernel::update::etat().map_err(|_| Error::NotFound)?;
    let etat = |s| match e.enregistrement.slot(s).etat {
        bouchaud_update_core::Etat::Vide => 0,
        bouchaud_update_core::Etat::Candidat { .. } => 1,
        bouchaud_update_core::Etat::Bon => 2,
        bouchaud_update_core::Etat::Mauvais => 3,
    };
    use bouchaud_update_core::Slot;
    let info = UpdateState {
        booted_slot: e.passage.slot as u32,
        boot_reason: e.passage.raison as u32,
        attempts_left: e.passage.essais_restants as u32,
        active_slot: e.enregistrement.actif as u32,
        state_a: etat(Slot::A),
        state_b: etat(Slot::B),
        meta_sequence: e.enregistrement.sequence,
        boot_id: e.passage.boot_id,
    };
    super::usercopy::write(args[1], &info.bytes_le())?;
    Ok(0)
}

/// 0 = the booted candidate is committed (durable), 1 = already the good
/// active slot. A refusal (no handoff, record changed, no real flush, ...)
/// is ACCESS_DENIED for the policy cases and NOT_FOUND when there is no A/B
/// layout at all.
fn update_commit_health(args: [u64; 6]) -> Result<u64> {
    handle::current_table().lookup_kind(HandleId::from_raw(args[0]), Rights::UPDATE_HEALTH, ObjectKind::Update)?;
    match crate::kernel::update::commit_sante() {
        Ok(crate::kernel::update::Commit::Committe) => Ok(0),
        Ok(crate::kernel::update::Commit::DejaSain) => Ok(1),
        Err("sans_passage") | Err("meta_introuvable") | Err("meta_sans_identite") => Err(Error::NotFound),
        Err(_) => Err(Error::AccessDenied),
    }
}

fn refus_install(r: crate::kernel::update::RefusInstall) -> Error {
    match r {
        crate::kernel::update::RefusInstall::Refuse(_) => Error::AccessDenied,
        crate::kernel::update::RefusInstall::Argument(_) => Error::InvalidArgument,
        // The device failed: reported as a fault of the operation.
        crate::kernel::update::RefusInstall::Io(_) => Error::Fault,
    }
}

fn update_install_begin(args: [u64; 6]) -> Result<u64> {
    handle::current_table().lookup_kind(HandleId::from_raw(args[0]), Rights::UPDATE_INSTALL, ObjectKind::Update)?;
    let cible = match args[1] { 0 => bouchaud_update_core::Slot::A, 1 => bouchaud_update_core::Slot::B,
                                _ => return Err(Error::InvalidArgument) };
    // args[2..4]: the signed manifest (canonical bytes, <= 1 KiB) and its
    // 64-byte Ed25519 signature.
    let len = args[3] as usize;
    if len == 0 || len > 1024 { return Err(Error::InvalidArgument); }
    let texte = super::usercopy::read(args[2], len)?;
    let brut = super::usercopy::read(args[4], 64)?;
    let mut signature = [0u8; 64];
    signature.copy_from_slice(&brut);
    crate::kernel::update::installation_commence(cible, &texte, &signature).map_err(refus_install)?;
    Ok(0)
}

/// At most 1 MiB per call: bounded kernel copy.
fn update_install_write(args: [u64; 6]) -> Result<u64> {
    handle::current_table().lookup_kind(HandleId::from_raw(args[0]), Rights::UPDATE_INSTALL, ObjectKind::Update)?;
    let len = args[3] as usize;
    if len == 0 || len > 1024 * 1024 { return Err(Error::TooLarge); }
    let donnees = super::usercopy::read(args[2], len)?;
    crate::kernel::update::installation_ecrit(args[1], &donnees).map_err(refus_install)?;
    Ok(0)
}

fn update_install_verify(args: [u64; 6]) -> Result<u64> {
    handle::current_table().lookup_kind(HandleId::from_raw(args[0]), Rights::UPDATE_INSTALL, ObjectKind::Update)?;
    crate::kernel::update::installation_verifie(args[1]).map_err(refus_install)
}

fn update_install_finish(args: [u64; 6]) -> Result<u64> {
    handle::current_table().lookup_kind(HandleId::from_raw(args[0]), Rights::UPDATE_INSTALL, ObjectKind::Update)?;
    let brut = super::usercopy::read(args[1], 32)?;
    let mut empreinte = [0u8; 32];
    empreinte.copy_from_slice(&brut);
    let essais = u8::try_from(args[3]).map_err(|_| Error::InvalidArgument)?;
    crate::kernel::update::installation_termine(empreinte, args[2], essais).map_err(refus_install)?;
    Ok(0)
}

fn process_kill(args: [u64; 6]) -> Result<u64> {
    let entry = handle::current_table().lookup_kind(
        HandleId::from_raw(args[0]), Rights::SIGNAL, ObjectKind::Process
    )?;
    match entry.object.as_ref() {
        Object::Process(p) => {
            if !p.terminated() {
                crate::kernel::task::tue_processus(p.pid(), 137);
            }
            Ok(0)
        }
        _ => Err(Error::WrongType),
    }
}

fn dispatch_inner(number: u64, args: [u64; 6]) -> Result<u64> {
    match number {
        numbers::VERSION => Ok(ABI_VERSION_PACKED),

        numbers::HANDLE_CLOSE => {
            handle::current_table().close(HandleId::from_raw(args[0]))?;
            Ok(0)
        }
        numbers::HANDLE_DUP => {
            let requested = Rights(args[1] as u32);
            let id = handle::current_table().duplicate(HandleId::from_raw(args[0]), requested)?;
            Ok(id.raw())
        }
        numbers::HANDLE_INFO => handle_info(args),
        numbers::HANDLE_COUNT => Ok(handle::current_table().count() as u64),

        numbers::CHANNEL_CREATE => pair_to_user(args[0]),
        numbers::CHANNEL_SEND => channel_send(args),
        numbers::CHANNEL_SEND_ATTENUE => channel_send_avec_masques(args, args[5]),
        numbers::CHANNEL_RECV => channel_recv(args),

        numbers::EVENT_CREATE => {
            let id = handle::install(Object::Event(Event::new(args[0] != 0)), Rights::EVENT_DEFAULT)?;
            Ok(id.raw())
        }
        numbers::EVENT_SIGNAL => {
            let entry = handle::current_table().lookup_kind(
                HandleId::from_raw(args[0]), Rights::SIGNAL, ObjectKind::Event
            )?;
            match entry.object.as_ref() {
                Object::Event(event) => { event.signal(); Ok(0) }
                _ => Err(Error::WrongType),
            }
        }
        numbers::EVENT_RESET => {
            let entry = handle::current_table().lookup_kind(
                HandleId::from_raw(args[0]), Rights::SIGNAL, ObjectKind::Event
            )?;
            match entry.object.as_ref() {
                Object::Event(event) => { event.reset(); Ok(0) }
                _ => Err(Error::WrongType),
            }
        }
        numbers::EVENT_QUERY => event_query(args),

        numbers::WAITSET_CREATE => {
            let id = handle::install(Object::WaitSet(WaitSet::new()), Rights::WAITSET_DEFAULT)?;
            Ok(id.raw())
        }
        numbers::WAITSET_ADD => {
            let table = handle::current_table();
            let waitset = table.lookup_kind(
                HandleId::from_raw(args[0]), Rights::WRITE, ObjectKind::WaitSet
            )?;
            let target = table.lookup(HandleId::from_raw(args[1]), Rights::WAIT)?;
            match waitset.object.as_ref() {
                Object::WaitSet(waitset) => {
                    waitset.add(args[2], Arc::clone(&target.object))?;
                    Ok(0)
                }
                _ => Err(Error::WrongType),
            }
        }
        numbers::WAITSET_REMOVE => {
            let entry = handle::current_table().lookup_kind(
                HandleId::from_raw(args[0]), Rights::WRITE, ObjectKind::WaitSet
            )?;
            match entry.object.as_ref() {
                Object::WaitSet(waitset) => { waitset.remove(args[1])?; Ok(0) }
                _ => Err(Error::WrongType),
            }
        }
        numbers::WAITSET_POLL => waitset_poll(args),
        numbers::WAITSET_WAIT => waitset_wait(args),

        numbers::SHM_CREATE => {
            let region = SharedRegion::new(args[0] as usize)?;
            let id = handle::install(Object::SharedRegion(region), Rights::SHM_DEFAULT)?;
            Ok(id.raw())
        }
        numbers::SHM_SIZE => {
            let entry = handle::current_table().lookup_kind(
                HandleId::from_raw(args[0]), Rights::INSPECT, ObjectKind::SharedRegion
            )?;
            match entry.object.as_ref() {
                Object::SharedRegion(region) => Ok(region.len() as u64),
                _ => Err(Error::WrongType),
            }
        }
        numbers::SHM_READ => {
            let entry = handle::current_table().lookup_kind(
                HandleId::from_raw(args[0]), Rights::READ, ObjectKind::SharedRegion
            )?;
            match entry.object.as_ref() {
                Object::SharedRegion(region) => {
                    let data = region.read(args[1] as usize, args[3] as usize)?;
                    super::usercopy::write(args[2], &data)?;
                    Ok(data.len() as u64)
                }
                _ => Err(Error::WrongType),
            }
        }
        numbers::SHM_WRITE => {
            let entry = handle::current_table().lookup_kind(
                HandleId::from_raw(args[0]), Rights::WRITE, ObjectKind::SharedRegion
            )?;
            let offset = args[1] as usize;
            let length = args[3] as usize;
            match entry.object.as_ref() {
                Object::SharedRegion(region) => {
                    // ARCH_P1_SHM_PRECHECK: never copy user-controlled data
                    // for an out-of-bounds write to a fixed-size SHM object.
                    let end = offset.checked_add(length).ok_or(Error::InvalidArgument)?;
                    if end > region.len() { return Err(Error::InvalidArgument); }
                    let data = super::usercopy::read(args[2], length)?;
                    region.write(offset, &data)?;
                    Ok(data.len() as u64)
                }
                _ => Err(Error::WrongType),
            }
        }

        numbers::PROCESS_SPAWN => process_spawn(args),
        numbers::PROCESS_INFO => process_info(args),
        numbers::PROCESS_KILL => process_kill(args),

        numbers::TIME_NOW => Ok(crate::kernel::native::time::monotonic_ns() & (i64::MAX as u64)),

        numbers::UPDATE_STATE => update_state(args),
        numbers::UPDATE_COMMIT_HEALTH => update_commit_health(args),
        numbers::UPDATE_INSTALL_BEGIN => update_install_begin(args),
        numbers::UPDATE_INSTALL_WRITE => update_install_write(args),
        numbers::UPDATE_INSTALL_FINISH => update_install_finish(args),
        numbers::UPDATE_INSTALL_VERIFY => update_install_verify(args),

        _ => Err(Error::InvalidCall),
    }
}

pub fn handle(frame: &mut TrapFrame) {
    handle::registry::maintenance();
    let (number, args) = frame.syscall_args();
    let result = dispatch_inner(number, args);
    frame.rax = result_u64(result) as u64;
}

#[cfg(debug_assertions)]
pub fn self_check() {
    debug_assert!(numbers::is_native(numbers::VERSION));
    debug_assert!(!numbers::is_native(0));
    debug_assert_eq!(ABI_VERSION_PACKED, 0x0001_0001);
}
