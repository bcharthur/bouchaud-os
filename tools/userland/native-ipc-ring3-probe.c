/*
 * Bouchaud OS native IPC V1 — freestanding ring3 proof.
 *
 * No libc is linked.  Every BO_* operation below reaches the native syscall
 * namespace directly.  Linux write/exit_group are used only to expose the
 * verdict on the serial console and to terminate the process.
 */
typedef unsigned long u64;
typedef unsigned int u32;
typedef long i64;
typedef unsigned long usize;

#define BO_BASE                 0x424f0000UL
#define BO_VERSION              (BO_BASE + 0x00)
#define BO_HANDLE_CLOSE         (BO_BASE + 0x01)
#define BO_HANDLE_DUP           (BO_BASE + 0x02)
#define BO_HANDLE_INFO          (BO_BASE + 0x03)
#define BO_HANDLE_COUNT         (BO_BASE + 0x04)

#define BO_CHANNEL_CREATE       (BO_BASE + 0x10)
#define BO_CHANNEL_SEND         (BO_BASE + 0x11)
#define BO_CHANNEL_RECV         (BO_BASE + 0x12)

#define BO_EVENT_CREATE         (BO_BASE + 0x20)
#define BO_EVENT_SIGNAL         (BO_BASE + 0x21)
#define BO_EVENT_RESET          (BO_BASE + 0x22)
#define BO_EVENT_QUERY          (BO_BASE + 0x23)

#define BO_WAITSET_CREATE       (BO_BASE + 0x30)
#define BO_WAITSET_ADD          (BO_BASE + 0x31)
#define BO_WAITSET_REMOVE       (BO_BASE + 0x32)
#define BO_WAITSET_POLL         (BO_BASE + 0x33)

#define BO_SHM_CREATE           (BO_BASE + 0x40)
#define BO_SHM_SIZE             (BO_BASE + 0x41)
#define BO_SHM_READ             (BO_BASE + 0x42)
#define BO_SHM_WRITE            (BO_BASE + 0x43)

#define BO_RIGHT_READ           (1u << 0)
#define BO_RIGHT_WRITE          (1u << 1)
#define BO_RIGHT_SIGNAL         (1u << 2)
#define BO_RIGHT_MAP            (1u << 3)
#define BO_RIGHT_DUP            (1u << 4)
#define BO_RIGHT_TRANSFER       (1u << 5)
#define BO_RIGHT_INSPECT        (1u << 6)
#define BO_RIGHT_WAIT           (1u << 7)

#define BO_SIGNAL_SIGNALED      (1u << 2)

#define BO_E_ACCESS_DENIED      5

struct bo_recv_meta {
    u64 bytes;
    u64 handles;
};

struct bo_wait_event {
    u64 key;
    u32 signals;
    u32 reserved;
};

static inline i64 sc6(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f)
{
    i64 r;
    register i64 r10 __asm__("r10") = d;
    register i64 r8  __asm__("r8")  = e;
    register i64 r9  __asm__("r9")  = f;
    __asm__ volatile(
        "syscall"
        : "=a"(r)
        : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return r;
}

static usize slen(const char *s)
{
    usize n = 0;
    while (s[n]) n++;
    return n;
}

static void write_text(const char *s)
{
    sc6(1, 1, (i64)s, (i64)slen(s), 0, 0, 0);
}

static void write_num(i64 v)
{
    char b[32];
    int i = 31;
    int neg = v < 0;
    b[i--] = 0;
    if (v == 0) b[i--] = '0';
    if (neg) v = -v;
    while (v) {
        b[i--] = (char)('0' + (v % 10));
        v /= 10;
    }
    if (neg) b[i--] = '-';
    write_text(&b[i + 1]);
}

static int bytes_equal(const unsigned char *a, const unsigned char *b, usize n)
{
    usize i;
    for (i = 0; i < n; i++) {
        if (a[i] != b[i]) return 0;
    }
    return 1;
}

__attribute__((noreturn))
static void finish(int code)
{
    sc6(231, code, 0, 0, 0, 0, 0); /* exit_group */
    for (;;) __asm__ volatile("hlt");
}

__attribute__((noreturn))
static void fail(const char *step, i64 code)
{
    write_text("[NATIVE-IPC-RING3] FAIL step=");
    write_text(step);
    write_text(" code=");
    write_num(code);
    write_text("\n");
    finish(1);
}

static void need_nonneg(const char *step, i64 value)
{
    if (value < 0) fail(step, value);
}

__attribute__((noinline, used, noreturn))
static void probe_main(void)
{
    i64 version;
    u64 pair[2] = {0, 0};
    static const unsigned char ping[] = "ping-bouchaud-native";
    unsigned char data[64];
    struct bo_recv_meta meta;
    i64 got;
    i64 event;
    u64 outgoing[1];
    u64 incoming[2] = {0, 0};
    static const unsigned char evt_payload[] = "evt";
    i64 received_event;
    i64 waitset;
    struct bo_wait_event ready[2];
    i64 ready_count;
    i64 wait_only;
    i64 denied;
    i64 shm;
    static const unsigned char shared[] = "shared-region-ring3-ok";
    unsigned char shared_out[64];
    i64 i;

    write_text("[NATIVE-IPC-RING3] BEGIN\n");

    version = sc6(BO_VERSION, 0, 0, 0, 0, 0, 0);
    need_nonneg("version", version);
    if (version != 0x00010000) fail("version-mismatch", version);
    write_text("[NATIVE-IPC-RING3] ABI=1.0\n");

    if (sc6(BO_CHANNEL_CREATE, (i64)pair, 0, 0, 0, 0, 0) != 0)
        fail("channel-create", -1);
    if (pair[0] == 0 || pair[1] == 0 || pair[0] == pair[1])
        fail("channel-handles", -1);

    if (sc6(BO_CHANNEL_SEND, pair[0], (i64)ping, sizeof(ping), 0, 0, 0)
            != (i64)sizeof(ping))
        fail("channel-send", -1);

    for (i = 0; i < (i64)sizeof(data); i++) data[i] = 0;
    meta.bytes = 0;
    meta.handles = 0;
    got = sc6(
        BO_CHANNEL_RECV, pair[1], (i64)data, sizeof(data),
        0, 0, (i64)&meta
    );
    need_nonneg("channel-recv", got);
    if (got != (i64)sizeof(ping)
        || meta.bytes != sizeof(ping)
        || meta.handles != 0
        || !bytes_equal(data, ping, sizeof(ping)))
        fail("channel-payload", got);

    write_text("[NATIVE-IPC-RING3] CHANNEL_OK\n");

    event = sc6(BO_EVENT_CREATE, 0, 0, 0, 0, 0, 0);
    need_nonneg("event-create", event);

    outgoing[0] = (u64)event;
    if (sc6(
            BO_CHANNEL_SEND, pair[0], (i64)evt_payload, sizeof(evt_payload),
            (i64)outgoing, 1, 0
        ) != (i64)sizeof(evt_payload))
        fail("handle-send", -1);

    for (i = 0; i < (i64)sizeof(data); i++) data[i] = 0;
    meta.bytes = 0;
    meta.handles = 0;
    got = sc6(
        BO_CHANNEL_RECV, pair[1], (i64)data, sizeof(data),
        (i64)incoming, 2, (i64)&meta
    );
    need_nonneg("handle-recv", got);
    if (meta.handles != 1 || incoming[0] == 0)
        fail("handle-transfer-count", (i64)meta.handles);
    received_event = (i64)incoming[0];

    if (sc6(BO_EVENT_SIGNAL, received_event, 0, 0, 0, 0, 0) != 0)
        fail("event-signal-transferred", -1);
    i = sc6(BO_EVENT_QUERY, event, 0, 0, 0, 0, 0);
    need_nonneg("event-query-original", i);
    if ((i & 1) == 0)
        fail("transferred-object-not-shared", i);

    write_text("[NATIVE-IPC-RING3] HANDLE_TRANSFER_OK\n");

    waitset = sc6(BO_WAITSET_CREATE, 0, 0, 0, 0, 0, 0);
    need_nonneg("waitset-create", waitset);
    if (sc6(BO_WAITSET_ADD, waitset, received_event, 0xC711, 0, 0, 0) != 0)
        fail("waitset-add", -1);

    ready[0].key = ready[1].key = 0;
    ready[0].signals = ready[1].signals = 0;
    ready_count = sc6(BO_WAITSET_POLL, waitset, (i64)ready, 2, 0, 0, 0);
    need_nonneg("waitset-poll", ready_count);
    if (ready_count < 1
        || ready[0].key != 0xC711
        || (ready[0].signals & BO_SIGNAL_SIGNALED) == 0)
        fail("waitset-event", ready_count);

    write_text("[NATIVE-IPC-RING3] EVENT_WAITSET_OK\n");

    wait_only = sc6(BO_HANDLE_DUP, event, BO_RIGHT_WAIT, 0, 0, 0, 0);
    need_nonneg("handle-dup-wait-only", wait_only);
    denied = sc6(BO_EVENT_SIGNAL, wait_only, 0, 0, 0, 0, 0);
    if (denied != -BO_E_ACCESS_DENIED)
        fail("rights-signal-must-be-denied", denied);

    write_text("[NATIVE-IPC-RING3] RIGHTS_OK\n");

    shm = sc6(BO_SHM_CREATE, 4096, 0, 0, 0, 0, 0);
    need_nonneg("shm-create", shm);
    i = sc6(BO_SHM_SIZE, shm, 0, 0, 0, 0, 0);
    if (i != 4096) fail("shm-size", i);

    if (sc6(BO_SHM_WRITE, shm, 128, (i64)shared, sizeof(shared), 0, 0)
            != (i64)sizeof(shared))
        fail("shm-write", -1);

    for (i = 0; i < (i64)sizeof(shared_out); i++) shared_out[i] = 0;
    if (sc6(BO_SHM_READ, shm, 128, (i64)shared_out, sizeof(shared), 0, 0)
            != (i64)sizeof(shared))
        fail("shm-read", -1);
    if (!bytes_equal(shared, shared_out, sizeof(shared)))
        fail("shm-payload", -1);

    write_text("[NATIVE-IPC-RING3] SHM_OK\n");

    /* Lifecycle/close: all process-local handles created above are closable. */
    sc6(BO_HANDLE_CLOSE, wait_only, 0, 0, 0, 0, 0);
    sc6(BO_HANDLE_CLOSE, waitset, 0, 0, 0, 0, 0);
    sc6(BO_HANDLE_CLOSE, received_event, 0, 0, 0, 0, 0);
    sc6(BO_HANDLE_CLOSE, event, 0, 0, 0, 0, 0);
    sc6(BO_HANDLE_CLOSE, shm, 0, 0, 0, 0, 0);
    sc6(BO_HANDLE_CLOSE, pair[0], 0, 0, 0, 0, 0);
    sc6(BO_HANDLE_CLOSE, pair[1], 0, 0, 0, 0, 0);

    write_text("[NATIVE-IPC-RING3] OK\n");
    finish(0);
}

/*
 * The kernel enters an ELF image directly at e_entry: there is no CALL and
 * therefore no synthetic return address on the initial user stack. GCC, on
 * the other hand, compiles an ordinary C function assuming the SysV function
 * entry convention (RSP % 16 == 8). A naked shim normalizes the raw ELF entry
 * stack and then CALLs the real C function, giving it exactly that ABI.
 */
__attribute__((naked, used, noreturn))
void _start(void)
{
    __asm__ volatile(
        "andq $-16, %rsp\n\t"
        "call probe_main\n\t"
        "ud2\n\t"
    );
}
