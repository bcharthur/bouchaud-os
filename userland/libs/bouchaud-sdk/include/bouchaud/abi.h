/*
 * Bouchaud native ABI -- the ONE C description of the kernel contract.
 *
 * Mirror of src/kernel/native/abi/{types,numbers,wire}.rs. Any change here
 * must be made there too; tools/ci/verifie_sdk_abi.py compares both sides and
 * fails the build on divergence.
 *
 * Freestanding: no libc type, no libc header.
 */
#ifndef BOUCHAUD_ABI_H
#define BOUCHAUD_ABI_H

typedef unsigned long      bo_u64;
typedef unsigned int       bo_u32;
typedef unsigned short     bo_u16;
typedef unsigned char      bo_u8;
typedef long               bo_i64;
typedef int                bo_i32;
typedef unsigned long      bo_usize;

/* ABI version: major << 16 | minor. */
#define BO_ABI_MAJOR 1
#define BO_ABI_MINOR 3

#define BO_NATIVE_BASE            0x424f0000UL
#define BO_NR_VERSION             (BO_NATIVE_BASE + 0x00)
#define BO_NR_HANDLE_CLOSE        (BO_NATIVE_BASE + 0x01)
#define BO_NR_HANDLE_DUP          (BO_NATIVE_BASE + 0x02)
#define BO_NR_HANDLE_INFO         (BO_NATIVE_BASE + 0x03)
#define BO_NR_HANDLE_COUNT        (BO_NATIVE_BASE + 0x04)
#define BO_NR_CHANNEL_CREATE      (BO_NATIVE_BASE + 0x10)
#define BO_NR_CHANNEL_SEND        (BO_NATIVE_BASE + 0x11)
#define BO_NR_CHANNEL_RECV        (BO_NATIVE_BASE + 0x12)
#define BO_NR_CHANNEL_SEND_ATTENUE (BO_NATIVE_BASE + 0x13)
#define BO_NR_EVENT_CREATE        (BO_NATIVE_BASE + 0x20)
#define BO_NR_EVENT_SIGNAL        (BO_NATIVE_BASE + 0x21)
#define BO_NR_EVENT_RESET         (BO_NATIVE_BASE + 0x22)
#define BO_NR_EVENT_QUERY         (BO_NATIVE_BASE + 0x23)
#define BO_NR_WAITSET_CREATE      (BO_NATIVE_BASE + 0x30)
#define BO_NR_WAITSET_ADD         (BO_NATIVE_BASE + 0x31)
#define BO_NR_WAITSET_REMOVE      (BO_NATIVE_BASE + 0x32)
#define BO_NR_WAITSET_POLL        (BO_NATIVE_BASE + 0x33)
#define BO_NR_WAITSET_WAIT        (BO_NATIVE_BASE + 0x34)
#define BO_NR_SHM_CREATE          (BO_NATIVE_BASE + 0x40)
#define BO_NR_SHM_SIZE            (BO_NATIVE_BASE + 0x41)
#define BO_NR_SHM_READ            (BO_NATIVE_BASE + 0x42)
#define BO_NR_SHM_WRITE           (BO_NATIVE_BASE + 0x43)
#define BO_NR_PROCESS_SPAWN       (BO_NATIVE_BASE + 0x50)
#define BO_NR_PROCESS_INFO        (BO_NATIVE_BASE + 0x51)
#define BO_NR_PROCESS_KILL        (BO_NATIVE_BASE + 0x52)
#define BO_NR_TIME_NOW            (BO_NATIVE_BASE + 0x60)
#define BO_NR_UPDATE_STATE        (BO_NATIVE_BASE + 0x70)
#define BO_NR_UPDATE_COMMIT_HEALTH (BO_NATIVE_BASE + 0x71)
#define BO_NR_UPDATE_INSTALL_BEGIN (BO_NATIVE_BASE + 0x72)
#define BO_NR_UPDATE_INSTALL_WRITE (BO_NATIVE_BASE + 0x73)
#define BO_NR_UPDATE_INSTALL_FINISH (BO_NATIVE_BASE + 0x74)
#define BO_NR_UPDATE_INSTALL_VERIFY (BO_NATIVE_BASE + 0x75)
#define BO_NR_DISPLAY_PRESENT     (BO_NATIVE_BASE + 0x80)

/* Errors are returned as negative values: -BO_E_xxx. */
#define BO_E_INVALID_CALL      1
#define BO_E_INVALID_ARGUMENT  2
#define BO_E_BAD_HANDLE        3
#define BO_E_WRONG_TYPE        4
#define BO_E_ACCESS_DENIED     5
#define BO_E_WOULD_BLOCK       6
#define BO_E_BUFFER_TOO_SMALL  7
#define BO_E_QUEUE_FULL        8
#define BO_E_PEER_CLOSED       9
#define BO_E_NO_SPACE          10
#define BO_E_FAULT             11
#define BO_E_TOO_LARGE         12
#define BO_E_NOT_FOUND         13
#define BO_E_TIMED_OUT         14
#define BO_E_DEVICE            15

/* Object kinds (bo_handle_info.kind). */
#define BO_KIND_CHANNEL        1
#define BO_KIND_EVENT          2
#define BO_KIND_WAITSET        3
#define BO_KIND_SHARED_REGION  4
#define BO_KIND_PROCESS        5
#define BO_KIND_UPDATE         6
#define BO_KIND_DISPLAY        7

/* Rights. */
#define BO_RIGHT_READ          (1u << 0)
#define BO_RIGHT_WRITE         (1u << 1)
#define BO_RIGHT_SIGNAL        (1u << 2)
#define BO_RIGHT_MAP           (1u << 3)
#define BO_RIGHT_DUP           (1u << 4)
#define BO_RIGHT_TRANSFER      (1u << 5)
#define BO_RIGHT_INSPECT       (1u << 6)
#define BO_RIGHT_WAIT          (1u << 7)
#define BO_RIGHT_UPDATE_HEALTH (1u << 8)
#define BO_RIGHT_UPDATE_INSTALL (1u << 9)
#define BO_RIGHT_DISPLAY_PRESENT (1u << 10)
#define BO_RIGHTS_ALL          0x7ffu

/* Signals. */
#define BO_SIGNAL_READABLE     (1u << 0)
#define BO_SIGNAL_WRITABLE     (1u << 1)
#define BO_SIGNAL_SIGNALED     (1u << 2)
#define BO_SIGNAL_PEER_CLOSED  (1u << 3)
#define BO_SIGNAL_TERMINATED   (1u << 4)

/* Limits. */
#define BO_MAX_MESSAGE_BYTES   (256 * 1024)
#define BO_MAX_MESSAGE_HANDLES 16
#define BO_MAX_SPAWN_HANDLES   8
#define BO_MAX_SPAWN_PATH      255

/* Process states (bo_process_info.state). */
#define BO_PROCESS_RUNNING     1
#define BO_PROCESS_TERMINATED  2

struct bo_recv_meta     { bo_u64 bytes; bo_u64 handles; };
struct bo_handle_info   { bo_u32 kind; bo_u32 rights; bo_u32 signals; bo_u32 reserved; };
struct bo_wait_event    { bo_u64 key; bo_u32 signals; bo_u32 reserved; };
struct bo_spawn_result  { bo_u64 pid; bo_u64 process; };
struct bo_process_info  { bo_u64 pid; bo_u32 state; bo_i32 exit_code; };
/* bo_update_state (ABI 1.2). Slots 0 = A, 1 = B; states 0 empty, 1 candidate,
 * 2 good, 3 bad; boot_reason as in system/update-core/src/passage.rs. */
struct bo_update_state  { bo_u32 booted_slot; bo_u32 boot_reason; bo_u32 attempts_left;
                          bo_u32 active_slot; bo_u32 state_a; bo_u32 state_b;
                          bo_u64 meta_sequence; bo_u64 boot_id; };

#endif
