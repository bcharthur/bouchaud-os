#ifndef BOUCHAUD_NATIVE_H
#define BOUCHAUD_NATIVE_H

#include <stddef.h>
#include <stdint.h>

#define BO_NATIVE_BASE        0x424f0000UL
#define BO_VERSION            (BO_NATIVE_BASE + 0x00)
#define BO_HANDLE_CLOSE       (BO_NATIVE_BASE + 0x01)
#define BO_HANDLE_DUP         (BO_NATIVE_BASE + 0x02)
#define BO_HANDLE_INFO        (BO_NATIVE_BASE + 0x03)
#define BO_HANDLE_COUNT       (BO_NATIVE_BASE + 0x04)
#define BO_CHANNEL_CREATE     (BO_NATIVE_BASE + 0x10)
#define BO_CHANNEL_SEND       (BO_NATIVE_BASE + 0x11)
#define BO_CHANNEL_RECV       (BO_NATIVE_BASE + 0x12)
#define BO_EVENT_CREATE       (BO_NATIVE_BASE + 0x20)
#define BO_EVENT_SIGNAL       (BO_NATIVE_BASE + 0x21)
#define BO_EVENT_RESET        (BO_NATIVE_BASE + 0x22)
#define BO_EVENT_QUERY        (BO_NATIVE_BASE + 0x23)
#define BO_WAITSET_CREATE     (BO_NATIVE_BASE + 0x30)
#define BO_WAITSET_ADD        (BO_NATIVE_BASE + 0x31)
#define BO_WAITSET_REMOVE     (BO_NATIVE_BASE + 0x32)
#define BO_WAITSET_POLL       (BO_NATIVE_BASE + 0x33)
#define BO_SHM_CREATE         (BO_NATIVE_BASE + 0x40)
#define BO_SHM_SIZE           (BO_NATIVE_BASE + 0x41)
#define BO_SHM_READ           (BO_NATIVE_BASE + 0x42)
#define BO_SHM_WRITE          (BO_NATIVE_BASE + 0x43)

#define BO_RIGHT_READ         (1u << 0)
#define BO_RIGHT_WRITE        (1u << 1)
#define BO_RIGHT_SIGNAL       (1u << 2)
#define BO_RIGHT_MAP          (1u << 3)
#define BO_RIGHT_DUP          (1u << 4)
#define BO_RIGHT_TRANSFER     (1u << 5)
#define BO_RIGHT_INSPECT      (1u << 6)
#define BO_RIGHT_WAIT         (1u << 7)

struct bo_recv_meta {
    uint64_t bytes;
    uint64_t handles;
};

struct bo_handle_info {
    uint32_t kind;
    uint32_t rights;
    uint32_t signals;
    uint32_t reserved;
};

struct bo_wait_event {
    uint64_t key;
    uint32_t signals;
    uint32_t reserved;
};

static inline long bo_syscall6(long nr, long a, long b, long c, long d, long e, long f)
{
    register long r10 __asm__("r10") = d;
    register long r8  __asm__("r8")  = e;
    register long r9  __asm__("r9")  = f;
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory");
    return ret;
}

static inline long bo_version(void) {
    return bo_syscall6(BO_VERSION, 0, 0, 0, 0, 0, 0);
}
static inline long bo_channel_create(uint64_t pair[2]) {
    return bo_syscall6(BO_CHANNEL_CREATE, (long)pair, 0, 0, 0, 0, 0);
}
static inline long bo_channel_send(uint64_t h, const void *data, size_t len,
                                   const uint64_t *handles, size_t handle_count) {
    return bo_syscall6(BO_CHANNEL_SEND, h, (long)data, len,
                       (long)handles, handle_count, 0);
}
static inline long bo_channel_recv(uint64_t h, void *data, size_t cap,
                                   uint64_t *handles, size_t handles_cap,
                                   struct bo_recv_meta *meta) {
    return bo_syscall6(BO_CHANNEL_RECV, h, (long)data, cap,
                       (long)handles, handles_cap, (long)meta);
}
static inline long bo_event_create(int initial) {
    return bo_syscall6(BO_EVENT_CREATE, initial != 0, 0, 0, 0, 0, 0);
}
static inline long bo_event_signal(uint64_t h) {
    return bo_syscall6(BO_EVENT_SIGNAL, h, 0, 0, 0, 0, 0);
}
static inline long bo_shm_create(size_t bytes) {
    return bo_syscall6(BO_SHM_CREATE, bytes, 0, 0, 0, 0, 0);
}
static inline long bo_shm_write(uint64_t h, size_t off, const void *data, size_t len) {
    return bo_syscall6(BO_SHM_WRITE, h, off, (long)data, len, 0, 0);
}
static inline long bo_shm_read(uint64_t h, size_t off, void *data, size_t len) {
    return bo_syscall6(BO_SHM_READ, h, off, (long)data, len, 0, 0);
}

#endif
