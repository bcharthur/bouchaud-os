typedef unsigned long u64;
typedef long i64;
typedef unsigned long usize;

#define SYS_WRITE       1
#define SYS_CLOSE       3
#define SYS_MMAP        9
#define SYS_MPROTECT    10
#define SYS_SOCKET      41
#define SYS_KILL        62
#define SYS_UNLINK      87
#define SYS_MKDIRAT      258
#define SYS_UNLINKAT     263
#define SYS_GETUID      102
#define SYS_SETUID      105
#define SYS_GETEUID     107
#define SYS_PRCTL       157
#define SYS_TKILL       200
#define SYS_EXIT_GROUP  231
#define SYS_OPENAT      257

#define PROT_READ  1
#define PROT_WRITE 2
#define PROT_EXEC  4
#define MAP_SHARED 1
#define MAP_PRIVATE 2
#define MAP_ANON 0x20
#define AT_FDCWD (-100)
#define O_RDONLY 0
#define O_RDWR 2
#define O_CREAT 0x40
#define O_DIRECTORY 0x10000

#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39

#define EACCES 13
#define EPERM 1
#define ENOENT 2

#define BO_BASE        0x424f0000UL
#define BO_SHM_CREATE  (BO_BASE + 0x40)
#define BO_E_ACCESS_DENIED 5

static inline i64 sc6(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    i64 r;
    register i64 r10 __asm__("r10") = d;
    register i64 r8 __asm__("r8") = e;
    register i64 r9 __asm__("r9") = f;
    __asm__ volatile(
        "syscall"
        : "=a"(r)
        : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return r;
}

static usize slen(const char *s) {
    usize n = 0;
    while (s[n]) n++;
    return n;
}

static void wr(const char *s) {
    sc6(SYS_WRITE, 1, (i64)s, (i64)slen(s), 0, 0, 0);
}

static void num(i64 v) {
    char b[32];
    int i = 31;
    int neg = v < 0;
    b[i--] = 0;
    if (!v) b[i--] = '0';
    if (neg) v = -v;
    while (v) {
        b[i--] = (char)('0' + (v % 10));
        v /= 10;
    }
    if (neg) b[i--] = '-';
    wr(&b[i + 1]);
}

__attribute__((noreturn))
static void finish(int code) {
    sc6(SYS_EXIT_GROUP, code, 0, 0, 0, 0, 0);
    for (;;) __asm__ volatile("hlt");
}

__attribute__((noreturn))
static void fail(const char *step, i64 code) {
    wr("[SECURITY-RING3] FAIL step=");
    wr(step);
    wr(" code=");
    num(code);
    wr("\n");
    finish(1);
}

static void expect(const char *step, i64 got, i64 wanted) {
    if (got != wanted) fail(step, got);
}

__attribute__((used, noinline))
void probe_main(void) {
    i64 r;
    i64 page;

    wr("[SECURITY-RING3] BEGIN\n");

    // Structural W^X applies even to the initial system profile.
    r = sc6(
        SYS_MMAP, 0, 4096,
        PROT_READ | PROT_WRITE | PROT_EXEC,
        MAP_PRIVATE | MAP_ANON, -1, 0
    );
    expect("mmap-rwx", r, -EACCES);
    wr("[SECURITY-RING3] WX_MMAP_DENIED\n");

    page = sc6(
        SYS_MMAP, 0, 4096,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANON, -1, 0
    );
    if (page < 0) fail("mmap-rw", page);

    r = sc6(
        SYS_MPROTECT, page, 4096,
        PROT_READ | PROT_WRITE | PROT_EXEC,
        0, 0, 0
    );
    expect("mprotect-rwx", r, -EACCES);
    wr("[SECURITY-RING3] WX_MPROTECT_DENIED\n");

    // Create a root-owned victim before dropping identity. /tmp is sticky, so
    // uid 1000 must not be able to remove it later.
    r = sc6(
        SYS_OPENAT, AT_FDCWD, (i64)"/tmp/security-root-owned",
        O_CREAT | O_RDWR, 0600, 0, 0
    );
    if (r < 0) fail("create-root-temp", r);
    sc6(SYS_CLOSE, r, 0, 0, 0, 0, 0);

    // Deliberate permanent privilege drop.
    r = sc6(SYS_SETUID, 1000, 0, 0, 0, 0, 0);
    expect("setuid-1000", r, 0);
    expect("getuid-after-drop", sc6(SYS_GETUID, 0, 0, 0, 0, 0, 0), 1000);
    expect("geteuid-after-drop", sc6(SYS_GETEUID, 0, 0, 0, 0, 0, 0), 1000);
    wr("[SECURITY-RING3] SETUID_DROP_OK\n");

    r = sc6(SYS_SETUID, 0, 0, 0, 0, 0, 0);
    expect("setuid-regain-root", r, -EPERM);
    wr("[SECURITY-RING3] PRIV_ESC_DENIED\n");

    // Prefix tricks must not bypass the device rule.
    r = sc6(SYS_OPENAT, AT_FDCWD, (i64)"/dev/fb0", O_RDONLY, 0, 0, 0);
    expect("device-open", r, -EACCES);
    r = sc6(
        SYS_OPENAT, AT_FDCWD, (i64)"/tmp/../dev/fb0",
        O_RDONLY, 0, 0, 0
    );
    expect("device-canonical", r, -EACCES);
    wr("[SECURITY-RING3] DEVICE_DENIED\n");
    wr("[SECURITY-RING3] PATH_CANONICAL_OK\n");

    // *at(2) policy and backend must use the SAME directory base.  The old
    // backend ignored mkdirat/unlinkat dirfd, so a policy-approved /tmp path
    // could actually mutate cwd.
    {
        i64 tmpfd = sc6(
            SYS_OPENAT, AT_FDCWD, (i64)"/tmp",
            O_RDONLY | O_DIRECTORY, 0, 0, 0
        );
        if (tmpfd < 0) fail("open-tmp-dirfd", tmpfd);

        r = sc6(
            SYS_OPENAT, tmpfd, (i64)"../dev/fb0",
            O_RDONLY, 0, 0, 0
        );
        expect("openat-dirfd-canonical", r, -EACCES);
        wr("[SECURITY-RING3] DIRFD_CANONICAL_OK\n");

        r = sc6(
            SYS_MKDIRAT, tmpfd, (i64)"security-dirfd-ok",
            0700, 0, 0, 0
        );
        expect("mkdirat-tmp", r, 0);

        r = sc6(
            SYS_OPENAT, AT_FDCWD, (i64)"/tmp/security-dirfd-ok",
            O_RDONLY | O_DIRECTORY, 0, 0, 0
        );
        if (r < 0) fail("verify-mkdirat-target", r);
        sc6(SYS_CLOSE, r, 0, 0, 0, 0, 0);

        r = sc6(
            SYS_OPENAT, AT_FDCWD, (i64)"/security-dirfd-ok",
            O_RDONLY | O_DIRECTORY, 0, 0, 0
        );
        expect("verify-mkdirat-no-cwd-escape", r, -ENOENT);

        r = sc6(
            SYS_OPENAT, tmpfd, (i64)"security-unlinkat",
            O_CREAT | O_RDWR, 0600, 0, 0
        );
        if (r < 0) fail("create-unlinkat-victim", r);
        sc6(SYS_CLOSE, r, 0, 0, 0, 0, 0);

        r = sc6(SYS_UNLINKAT, tmpfd, (i64)"security-unlinkat", 0, 0, 0, 0);
        expect("unlinkat-tmp", r, 0);
        r = sc6(
            SYS_OPENAT, AT_FDCWD, (i64)"/tmp/security-unlinkat",
            O_RDONLY, 0, 0, 0
        );
        expect("verify-unlinkat-target", r, -ENOENT);
        sc6(SYS_CLOSE, tmpfd, 0, 0, 0, 0, 0);
        wr("[SECURITY-RING3] DIRFD_MUTATION_OK\n");
    }

    r = sc6(SYS_UNLINK, (i64)"/tmp/security-root-owned", 0, 0, 0, 0, 0);
    expect("sticky-unlink", r, -EACCES);
    wr("[SECURITY-RING3] STICKY_TMP_OK\n");

    // A writable FD does not override inode mode on a shared writable mapping.
    r = sc6(
        SYS_OPENAT, AT_FDCWD, (i64)"/tmp/security-map-ro",
        O_CREAT | O_RDWR, 0400, 0, 0
    );
    if (r < 0) fail("create-map-ro", r);
    {
        i64 mapped = sc6(
            SYS_MMAP, 0, 4096, PROT_WRITE,
            MAP_SHARED, r, 0
        );
        expect("mmap-shared-dac", mapped, -EACCES);
    }
    sc6(SYS_CLOSE, r, 0, 0, 0, 0, 0);
    wr("[SECURITY-RING3] MMAP_DAC_OK\n");

    r = sc6(SYS_SOCKET, 2 /* AF_INET */, 3 /* SOCK_RAW */, 0, 0, 0, 0);
    expect("raw-socket", r, -EPERM);
    wr("[SECURITY-RING3] RAW_SOCKET_DENIED\n");

    r = sc6(SYS_KILL, 1, 0, 0, 0, 0, 0);
    expect("signal-pid1", r, -EPERM);
    wr("[SECURITY-RING3] SIGNAL_DENIED\n");

    r = sc6(SYS_TKILL, 1, 0, 0, 0, 0, 0);
    expect("signal-thread", r, -EPERM);
    wr("[SECURITY-RING3] THREAD_SIGNAL_DENIED\n");

    r = sc6(SYS_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0, 0);
    expect("pr-set-nnp", r, 0);
    r = sc6(SYS_PRCTL, PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0, 0);
    expect("pr-get-nnp", r, 1);
    wr("[SECURITY-RING3] NNP_OK\n");

    // User profile: one native shared-region object is capped at 16 MiB.
    r = sc6(BO_SHM_CREATE, 32 * 1024 * 1024, 0, 0, 0, 0, 0);
    expect("native-shm-limit", r, -BO_E_ACCESS_DENIED);
    wr("[SECURITY-RING3] NATIVE_SHM_LIMIT_OK\n");

    // Anonymous RX is a JIT privilege. User profile does not have it.
    r = sc6(SYS_MPROTECT, page, 4096, PROT_READ | PROT_EXEC, 0, 0, 0);
    expect("anonymous-rx-no-jit", r, -EACCES);
    wr("[SECURITY-RING3] JIT_DENIED\n");

    wr("[SECURITY-RING3] OK\n");
    finish(0);
}

__attribute__((naked, noreturn))
void _start(void) {
    __asm__ volatile(
        "andq $-16, %rsp\n"
        "call probe_main\n"
        "ud2\n"
    );
}
