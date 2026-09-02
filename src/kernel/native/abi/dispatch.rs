use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::arch::x86_64::usermode::TrapFrame;

use super::numbers;
use super::types::{
    Error, HandleId, ObjectKind, Result, Rights, ABI_VERSION_PACKED,
};
use super::wire::{HandleInfo, RecvMeta, WaitEvent};
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

fn channel_send(args: [u64; 6]) -> Result<u64> {
    let handle_id = HandleId::from_raw(args[0]);
    let data_len = args[2] as usize;
    let handle_count = args[4] as usize;

    if data_len > MAX_MESSAGE_BYTES || handle_count > MAX_MESSAGE_HANDLES {
        return Err(Error::TooLarge);
    }

    let bytes = super::usercopy::read(args[1], data_len)?;
    let ids = super::usercopy::read_handles(args[3], handle_count)?;
    let table = handle::current_table();

    let endpoint = table.lookup_kind(handle_id, Rights::WRITE, ObjectKind::Channel)?;

    let mut transferred = Vec::with_capacity(ids.len());
    for id in ids {
        let entry = table.export(id)?;
        transferred.push(TransferredHandle { object: entry.object, rights: entry.rights });
    }

    match endpoint.object.as_ref() {
        Object::Channel(channel) => channel.send(Message::new(bytes, transferred)).map(|n| n as u64),
        _ => Err(Error::WrongType),
    }
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
        return Err(error);
    }
    if let Err(error) = super::usercopy::write_handles(args[3], &installed) {
        for id in installed { let _ = table.close(id); }
        return Err(error);
    }
    if meta_ptr != 0 {
        let meta = RecvMeta { bytes: message.bytes.len() as u64, handles: installed.len() as u64 };
        if let Err(error) = super::usercopy::write(meta_ptr, &meta.bytes_le()) {
            for id in installed { let _ = table.close(id); }
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
            let data = super::usercopy::read(args[2], args[3] as usize)?;
            match entry.object.as_ref() {
                Object::SharedRegion(region) => {
                    region.write(args[1] as usize, &data)?;
                    Ok(data.len() as u64)
                }
                _ => Err(Error::WrongType),
            }
        }

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
    debug_assert_eq!(ABI_VERSION_PACKED, 0x0001_0000);
}
