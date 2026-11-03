/*
 * bouchaud-sdk -- the native Bouchaud API for ring 3 programs.
 *
 * A native Bouchaud program includes this header and nothing else. It never
 * writes a raw syscall: every kernel object is reached through a bo_* call,
 * and every failure comes back as a negative BO_E_* value.
 *
 * Freestanding. The only Linux-compatible calls used are write(2) on fd 1,
 * for the console, and exit_group(2); both are isolated here so a future
 * native console/exit object can replace them without touching programs.
 *
 * Entry: a program defines
 *
 *     int bo_main(const struct bo_start *start);
 *
 * and links runtime.c, which provides _start, parses the initial stack and
 * extracts the bootstrap handles a supervisor passed with bo_process_spawn.
 */
#ifndef BOUCHAUD_SDK_H
#define BOUCHAUD_SDK_H

#include "abi.h"

/* ---- raw transport (private to the SDK) ------------------------------- */

static inline bo_i64 bo__sys6(bo_u64 nr, bo_u64 a, bo_u64 b, bo_u64 c,
                              bo_u64 d, bo_u64 e, bo_u64 f)
{
    register bo_u64 r10 __asm__("r10") = d;
    register bo_u64 r8  __asm__("r8")  = e;
    register bo_u64 r9  __asm__("r9")  = f;
    bo_i64 ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory");
    return ret;
}

typedef bo_u64 bo_handle_t;

/* ---- start-up ---------------------------------------------------------- */

#define BO_START_MAX_BOOTSTRAP BO_MAX_SPAWN_HANDLES

struct bo_start {
    int          argc;
    char       **argv;
    char       **envp;
    /* Handles a supervisor passed at spawn, in the order it passed them. */
    bo_handle_t  bootstrap[BO_START_MAX_BOOTSTRAP];
    int          bootstrap_count;
};

int bo_main(const struct bo_start *start);

/* ---- version / time ---------------------------------------------------- */

static inline bo_i64 bo_version(void)
{ return bo__sys6(BO_NR_VERSION, 0, 0, 0, 0, 0, 0); }

/* Monotonic nanoseconds since boot. Never negative on success. */
static inline bo_i64 bo_time_now_ns(void)
{ return bo__sys6(BO_NR_TIME_NOW, 0, 0, 0, 0, 0, 0); }

/* ---- handles ----------------------------------------------------------- */

static inline bo_i64 bo_handle_close(bo_handle_t h)
{ return bo__sys6(BO_NR_HANDLE_CLOSE, h, 0, 0, 0, 0, 0); }

static inline bo_i64 bo_handle_dup(bo_handle_t h, bo_u32 rights)
{ return bo__sys6(BO_NR_HANDLE_DUP, h, rights, 0, 0, 0, 0); }

static inline bo_i64 bo_handle_info(bo_handle_t h, struct bo_handle_info *out)
{ return bo__sys6(BO_NR_HANDLE_INFO, h, (bo_u64)out, 0, 0, 0, 0); }

static inline bo_i64 bo_handle_count(void)
{ return bo__sys6(BO_NR_HANDLE_COUNT, 0, 0, 0, 0, 0, 0); }

/* ---- channels ---------------------------------------------------------- */

static inline bo_i64 bo_channel_create(bo_handle_t pair[2])
{ return bo__sys6(BO_NR_CHANNEL_CREATE, (bo_u64)pair, 0, 0, 0, 0, 0); }

static inline bo_i64 bo_channel_send(bo_handle_t ch, const void *data, bo_usize len,
                                     const bo_handle_t *handles, bo_usize count)
{ return bo__sys6(BO_NR_CHANNEL_SEND, ch, (bo_u64)data, len, (bo_u64)handles, count, 0); }

/* Each transferred handle arrives with rights & masks[i]. */
static inline bo_i64 bo_channel_send_attenuated(bo_handle_t ch, const void *data, bo_usize len,
                                                const bo_handle_t *handles, bo_usize count,
                                                const bo_u32 *masks)
{ return bo__sys6(BO_NR_CHANNEL_SEND_ATTENUE, ch, (bo_u64)data, len, (bo_u64)handles, count, (bo_u64)masks); }

static inline bo_i64 bo_channel_recv(bo_handle_t ch, void *data, bo_usize cap,
                                     bo_handle_t *handles, bo_usize handles_cap,
                                     struct bo_recv_meta *meta)
{ return bo__sys6(BO_NR_CHANNEL_RECV, ch, (bo_u64)data, cap, (bo_u64)handles, handles_cap, (bo_u64)meta); }

/* ---- events ------------------------------------------------------------ */

static inline bo_i64 bo_event_create(int initially_signaled)
{ return bo__sys6(BO_NR_EVENT_CREATE, initially_signaled != 0, 0, 0, 0, 0, 0); }

static inline bo_i64 bo_event_signal(bo_handle_t ev)
{ return bo__sys6(BO_NR_EVENT_SIGNAL, ev, 0, 0, 0, 0, 0); }

static inline bo_i64 bo_event_reset(bo_handle_t ev)
{ return bo__sys6(BO_NR_EVENT_RESET, ev, 0, 0, 0, 0, 0); }

/* ---- wait sets --------------------------------------------------------- */

static inline bo_i64 bo_waitset_create(void)
{ return bo__sys6(BO_NR_WAITSET_CREATE, 0, 0, 0, 0, 0, 0); }

/* A wait set can watch channels, events, shared regions and processes, but
 * never another wait set (BO_E_WRONG_TYPE): nesting is how a cycle forms. */
static inline bo_i64 bo_waitset_add(bo_handle_t ws, bo_handle_t target, bo_u64 key)
{ return bo__sys6(BO_NR_WAITSET_ADD, ws, target, key, 0, 0, 0); }

static inline bo_i64 bo_waitset_remove(bo_handle_t ws, bo_u64 key)
{ return bo__sys6(BO_NR_WAITSET_REMOVE, ws, key, 0, 0, 0, 0); }

/* Non-blocking: returns the number of ready events written to out. */
static inline bo_i64 bo_waitset_poll(bo_handle_t ws, struct bo_wait_event *out, bo_usize cap)
{ return bo__sys6(BO_NR_WAITSET_POLL, ws, (bo_u64)out, cap, 0, 0, 0); }

/* Blocking, level-triggered: returns >0 ready events, or -BO_E_TIMED_OUT
 * after timeout_ns. timeout_ns == 0 behaves like bo_waitset_poll. */
static inline bo_i64 bo_waitset_wait(bo_handle_t ws, struct bo_wait_event *out, bo_usize cap,
                                     bo_u64 timeout_ns)
{ return bo__sys6(BO_NR_WAITSET_WAIT, ws, (bo_u64)out, cap, timeout_ns, 0, 0); }

/* ---- shared regions ---------------------------------------------------- */

static inline bo_i64 bo_shm_create(bo_usize bytes)
{ return bo__sys6(BO_NR_SHM_CREATE, bytes, 0, 0, 0, 0, 0); }

static inline bo_i64 bo_shm_size(bo_handle_t shm)
{ return bo__sys6(BO_NR_SHM_SIZE, shm, 0, 0, 0, 0, 0); }

static inline bo_i64 bo_shm_read(bo_handle_t shm, bo_usize off, void *out, bo_usize len)
{ return bo__sys6(BO_NR_SHM_READ, shm, off, (bo_u64)out, len, 0, 0); }

static inline bo_i64 bo_shm_write(bo_handle_t shm, bo_usize off, const void *data, bo_usize len)
{ return bo__sys6(BO_NR_SHM_WRITE, shm, off, (bo_u64)data, len, 0, 0); }

/* ---- processes --------------------------------------------------------- */

/* Launch `path` with `count` bootstrap handles, each attenuated to masks[i]
 * (masks may be 0: no attenuation). On success out->process is a handle on
 * the child (WAIT/INSPECT/SIGNAL) whose TERMINATED signal rises at its death. */
static inline bo_i64 bo_process_spawn(const char *path, bo_usize path_len,
                                      const bo_handle_t *handles, bo_usize count,
                                      const bo_u32 *masks, struct bo_spawn_result *out)
{ return bo__sys6(BO_NR_PROCESS_SPAWN, (bo_u64)path, path_len, (bo_u64)handles, count, (bo_u64)masks, (bo_u64)out); }

static inline bo_i64 bo_process_info(bo_handle_t proc, struct bo_process_info *out)
{ return bo__sys6(BO_NR_PROCESS_INFO, proc, (bo_u64)out, 0, 0, 0, 0); }

static inline bo_i64 bo_process_kill(bo_handle_t proc)
{ return bo__sys6(BO_NR_PROCESS_KILL, proc, 0, 0, 0, 0, 0); }

/* ---- console / exit (isolated Linux-compatible edge) ------------------- */

static inline bo_usize bo_strlen(const char *s)
{ bo_usize n = 0; while (s[n]) n++; return n; }

static inline void bo_write(const char *s, bo_usize n)
{ (void)bo__sys6(1 /* write */, 1, (bo_u64)s, n, 0, 0, 0); }

static inline void bo_puts(const char *s) { bo_write(s, bo_strlen(s)); }

/* The zombie of a spawned child belongs to its POSIX parent, as for a fork
 * child: a Process handle only OBSERVES (TERMINATED + exit code) and never
 * collects. A parent collects with wait4 (pid > 0: that child; -1: any).
 * Returns the pid, 0 with BO_WAIT_NOHANG if none is ready, or -errno
 * (-10 = ECHILD: not a child of the caller, or already collected). */
#define BO_WAIT_NOHANG 1
static inline bo_i64 bo_posix_wait4(bo_i64 pid, int *status, int options)
{ return bo__sys6(61 /* wait4 */, (bo_u64)pid, (bo_u64)status, (bo_u64)(bo_i64)options, 0, 0, 0); }

void bo_put_i64(bo_i64 v);
void bo_put_u64(bo_u64 v);

/* One console line = ONE write. Lines written piecewise get interleaved with
 * kernel log lines on the shared serial console, which corrupts markers a
 * bench must find whole. Build the line, then emit it once. */
#define BO_LINE_CAP 240
struct bo_line { bo_usize len; char buf[BO_LINE_CAP]; };
void bo_line_begin(struct bo_line *l);
void bo_line_str(struct bo_line *l, const char *s);
void bo_line_i64(struct bo_line *l, bo_i64 v);
void bo_line_kv(struct bo_line *l, const char *key, bo_i64 v);   /* " key=v" */
void bo_line_end(struct bo_line *l);                               /* adds '\n', writes */

/* Line-buffered console for single-threaded programs: text accumulates in a
 * process-wide line and is written once, at each '\n' (or when full). */
void bo_say(const char *s);
void bo_say_i64(bo_i64 v);
void bo_say_kv(const char *key, bo_i64 v);

__attribute__((noreturn)) void bo_exit(int code);

/* Native sleep built on a private wait set: never busy-waits. */
bo_i64 bo_sleep_ns(bo_u64 ns);

#endif
