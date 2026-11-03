/*
 * updated -- the Bouchaud update agent, ring 3 (BOUCHAUD_UPDATED_V1).
 *
 * Supervised by serviced like any service (hello/ready, ping/pong, stop). It
 * owns NO ambient authority. Its two bootstrap handles:
 *
 *   [0] its control channel                 READ | WRITE | WAIT
 *   [1] an update capability                INSPECT | UPDATE_INSTALL
 *       (no UPDATE_HEALTH: updated can write the INACTIVE slot and declare a
 *        candidate, it can never declare a boot healthy; no DUP/TRANSFER:
 *        it cannot hand the capability on)
 *
 * The kernel mechanism behind UPDATE_INSTALL accepts exactly three steps:
 *   begin(target)   target must be neither the running nor the active slot
 *   write(off,buf)  only inside the target slot's partition
 *   finish(sha,len) flush, read back, SHA-256 must match -> Candidate{N}
 *
 * Pending update, shipped inside the running system by the release pipeline:
 *   /update/manifest      the CANONICAL signed manifest (system/update-core
 *                         src/manifeste.rs): version, image_sha256,
 *                         image_octets, essais
 *   /update/manifest.sig  its 64-byte Ed25519 signature
 *   /update/system.img    the slot image the manifest names
 *   /update/selftest      (bench only) also exercise every refusal
 * Two verdicts, both decided by the KERNEL, never by this agent:
 *   SIGNATURE TRUSTED  at begin (a compiled-in release key verifies the
 *                      manifest bytes), and
 *   HASH VALID         at finish (the slot read back from disk matches what
 *                      the signed manifest names).
 */
#include "bouchaud/sdk.h"

#define KEY_CONTROL 1
/* 64 KiB: the first reads of a lazily materialised ramdisk file are slow
 * under TCG; at 256 KiB one read+write exceeded the supervisor's heartbeat
 * tolerance (4 x 250 ms) and updated was killed twice before converging
 * (QEMU bench mise-a-jour). The tolerance is not changed: the step is. */
#define CHUNK       (64 * 1024)
#define MANIFEST    "/update/manifest"
#define SIGNATURE   "/update/manifest.sig"
#define IMAGE       "/update/system.img"
#define SELFTEST    "/update/selftest"

static bo_handle_t control, cap;
static bo_handle_t ws;
static int failures;
static char chunk[CHUNK];

static int prefix(const char *s, bo_usize n, const char *p)
{
    bo_usize i = 0;
    while (p[i]) { if (i >= n || s[i] != p[i]) return 0; i++; }
    return 1;
}

static bo_usize put(char *d, bo_usize at, bo_usize cap_, const char *s, bo_usize n)
{
    bo_usize i;
    for (i = 0; i < n && at < cap_; i++) d[at++] = s[i];
    return at;
}

static bo_usize put_u64(char *d, bo_usize at, bo_usize cap_, bo_u64 v)
{
    char t[24]; int i = 24;
    if (!v) t[--i] = '0';
    while (v) { t[--i] = (char)('0' + v % 10); v /= 10; }
    return put(d, at, cap_, t + i, (bo_usize)(24 - i));
}

/* Answer the supervisor without blocking: called between every chunk, so a
 * long install never looks like a hang. Returns 0 when asked to stop. */
static int serve_control(bo_u64 timeout_ns)
{
    struct bo_wait_event ev[1];
    struct bo_recv_meta meta;
    char in[128], out[160];
    bo_i64 n = bo_waitset_wait(ws, ev, 1, timeout_ns), got;
    bo_usize len = 0;
    if (n <= 0) return 1;
    got = bo_channel_recv(control, in, sizeof(in), 0, 0, &meta);
    if (got == -BO_E_PEER_CLOSED) return 0;
    if (got < 0) return 1;
    if (prefix(in, (bo_usize)got, "ping ")) {
        len = put(out, 0, sizeof(out), "pong ", 5);
        len = put(out, len, sizeof(out), in + 5, (bo_usize)got - 5);
        len = put(out, len, sizeof(out), " ", 1);
        len = put_u64(out, len, sizeof(out), (bo_u64)bo_handle_count());
        len = put(out, len, sizeof(out), " ", 1);
        len = put_u64(out, len, sizeof(out), (bo_u64)bo_time_now_ns());
    } else if (prefix(in, (bo_usize)got, "hello ")) {
        len = put(out, 0, sizeof(out), "ready ", 6);
        len = put(out, len, sizeof(out), in + 6, (bo_usize)got - 6);
    } else if (prefix(in, (bo_usize)got, "stop")) {
        (void)bo_channel_send(control, "bye", 3, 0, 0);
        return 0;
    } else {
        len = put(out, 0, sizeof(out), "error", 5);
    }
    bo_channel_send(control, out, len, 0, 0);
    return 1;
}

static void check(const char *what, int ok, bo_i64 detail)
{
    struct bo_line l;
    bo_line_begin(&l);
    bo_line_str(&l, ok ? "[UPDATED] ok   " : "[UPDATED] FAIL ");
    bo_line_str(&l, what);
    bo_line_kv(&l, "detail", detail);
    bo_line_end(&l);
    if (!ok) failures++;
}

/* ---- manifest ------------------------------------------------------------ */

struct manifest {
    char   image[128];
    bo_u64 bytes;
    bo_u8  sha256[32];
    bo_u64 attempts;
    int    selftest;
    int    has_sha;
    char   raw[1024];          /* the exact signed bytes */
    bo_usize raw_len;
    bo_u8  sig[64];
};

static int hexval(char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    return -1;
}

static int parse_manifest(struct manifest *m)
{
    bo_i64 fd = bo_posix_open(MANIFEST, 0), n;
    bo_usize i = 0, k;
    const char *img = IMAGE;
    if (fd < 0) return 0;
    n = bo_posix_read(fd, m->raw, sizeof(m->raw));
    bo_posix_close(fd);
    if (n <= 0 || n >= (bo_i64)sizeof(m->raw)) return -1;
    m->raw_len = (bo_usize)n;
    fd = bo_posix_open(SIGNATURE, 0);
    if (fd < 0 || bo_posix_read(fd, m->sig, 64) != 64) { if (fd >= 0) bo_posix_close(fd); return -1; }
    bo_posix_close(fd);
    fd = bo_posix_open(SELFTEST, 0);
    if (fd >= 0) { m->selftest = 1; bo_posix_close(fd); }
    for (k = 0; img[k] && k + 1 < sizeof(m->image); k++) m->image[k] = img[k];
    m->image[k] = 0;
    /* Read what the manifest names, for this agent's own I/O. The KERNEL
     * parses the same bytes canonically and decides; nothing here is
     * trusted by it. */
    while (i < (bo_usize)n) {
        bo_usize s0 = i, e;
        const char *text = m->raw;
        while (i < (bo_usize)n && text[i] != '\n') i++;
        e = i++;
        if (prefix(text + s0, e - s0, "image_octets=") || prefix(text + s0, e - s0, "essais=")) {
            bo_usize at = s0; bo_u64 v = 0; int is_bytes = text[s0] == 'i';
            while (at < e && text[at] != '=') at++;
            for (at++; at < e && text[at] >= '0' && text[at] <= '9'; at++) v = v * 10 + (bo_u64)(text[at] - '0');
            if (is_bytes) m->bytes = v; else m->attempts = v;
        } else if (prefix(text + s0, e - s0, "image_sha256=") && e - s0 == 13 + 64) {
            for (k = 0; k < 32; k++) {
                int h = hexval(text[s0 + 13 + 2 * k]), l = hexval(text[s0 + 14 + 2 * k]);
                if (h < 0 || l < 0) return -1;
                m->sha256[k] = (bo_u8)(h << 4 | l);
            }
            m->has_sha = 1;
        }
    }
    return (m->bytes && m->has_sha && m->attempts) ? 1 : -1;
}

/* ---- refusals (DENY by default, each one observed) ------------------------ */

static void selftest_refusals(const struct manifest *m, int running_slot, int target)
{
    bo_i64 rc, i;
    int all_bad = 1;
    static char probe[4096];
    static char altered[1024];
    bo_u8 badsig[64];

    rc = bo_update_install_write(cap, 0, probe, sizeof(probe));
    check("write before begin is refused", rc == -BO_E_ACCESS_DENIED, rc);
    rc = bo_update_install_begin(cap, (bo_u32)running_slot, m->raw, m->raw_len, m->sig);
    check("begin on the RUNNING slot is refused (I6)", rc == -BO_E_ACCESS_DENIED, rc);
    /* SIGNATURE TRUSTED is the kernel's call: one flipped bit of the
     * signature, or one changed byte of the manifest, and begin is refused
     * before anything is read or written. */
    for (i = 0; i < 64; i++) badsig[i] = m->sig[i];
    badsig[5] ^= 0x10;
    rc = bo_update_install_begin(cap, (bo_u32)target, m->raw, m->raw_len, badsig);
    check("begin with a flipped signature bit is refused", rc == -BO_E_ACCESS_DENIED, rc);
    for (i = 0; i < (bo_i64)m->raw_len; i++) altered[i] = m->raw[i];
    altered[m->raw_len - 2] = altered[m->raw_len - 2] == '9' ? '8' : '9';   /* essais=N -> other N */
    rc = bo_update_install_begin(cap, (bo_u32)target, altered, m->raw_len, m->sig);
    check("begin with an altered manifest (more attempts) is refused", rc == -BO_E_ACCESS_DENIED, rc);
    rc = bo_update_commit_health(cap);
    check("install capability cannot declare health", rc == -BO_E_ACCESS_DENIED, rc);
    rc = bo_handle_dup(cap, BO_RIGHT_INSPECT);
    check("install capability cannot be duplicated", rc < 0, rc);
    /* Another service: updated holds no handle to any other process. Every
     * handle id it does not own is unusable. */
    for (i = 1; i <= 64; i++) {
        bo_handle_t h = (bo_handle_t)i;
        if (h == control || h == cap) continue;
        rc = bo_process_kill(h);
        if (rc != -BO_E_BAD_HANDLE && rc != -BO_E_WRONG_TYPE) all_bad = 0;
    }
    check("no handle reaches another service (kill on 64 foreign ids)", all_bad, 0);
    /* Its whole authority: control channel, capability, its own wait set. */
    rc = bo_handle_count();
    check("handle table is exactly control + capability + waitset", rc == 3, rc);
    /* Raw disks: no path names a block device for this process. */
    rc = bo_posix_open("/dev/nvme0n1", 2);
    check("raw disk /dev/nvme0n1 not reachable", rc < 0, rc);
    rc = bo_posix_open("/dev/sda", 2);
    check("raw disk /dev/sda not reachable", rc < 0, rc);
    (void)target;
}

static bo_u8 probe_sha[32];

static void selftest_after_begin(void)
{
    static char probe[4096];
    bo_i64 rc;
    /* Past the end of the target partition: the next partition is the STATE
     * (user data) or nothing. The kernel bounds every write to the slot. */
    rc = bo_update_install_write(cap, (bo_u64)1 << 40, probe, sizeof(probe));
    check("write outside the target slot (user data) is refused", rc == -BO_E_ACCESS_DENIED, rc);
    rc = bo_update_install_write(cap, 1, probe, sizeof(probe));
    check("unaligned write is refused", rc == -BO_E_INVALID_ARGUMENT, rc);
    rc = bo_update_install_finish(cap, probe_sha, 4096, 3);
    check("finish before a full read back is refused", rc == -BO_E_ACCESS_DENIED, rc);
}

/* ---- install --------------------------------------------------------------- */

static int install(const struct manifest *m, int target)
{
    bo_u64 off = 0;
    bo_i64 fd, rc;
    struct bo_line l;

    rc = bo_update_install_begin(cap, (bo_u32)target, m->raw, m->raw_len, m->sig);
    check("begin on the inactive slot: signed manifest trusted by the kernel", rc == 0, rc);
    if (rc != 0) return 0;
    if (m->selftest) selftest_after_begin();
    fd = bo_posix_open(m->image, 0);
    check("image readable", fd >= 0, fd);
    if (fd < 0) return 0;
    while (off < m->bytes) {
        bo_u64 want = m->bytes - off < CHUNK ? m->bytes - off : CHUNK;
        if (!serve_control(0)) { bo_posix_close(fd); return 0; }   /* before the read, too */
        bo_i64 got = bo_posix_read(fd, chunk, want), pad;
        if (got <= 0) { check("image shorter than its manifest", 0, (bo_i64)off); bo_posix_close(fd); return 0; }
        /* Block-aligned writes: the tail is zero-padded; the hash covers
         * exactly `bytes`. */
        for (pad = got; pad % 4096; pad++) chunk[pad] = 0;
        rc = bo_update_install_write(cap, off, chunk, (bo_u64)pad);
        if (rc != 0) { check("write into the inactive slot", 0, rc); bo_posix_close(fd); return 0; }
        off += (bo_u64)got;
        if (off % (16u << 20) < (bo_u64)got) {
            struct bo_line l;
            bo_line_begin(&l);
            bo_line_str(&l, "UPDATED_PROGRESS");
            bo_line_kv(&l, "written", (bo_i64)off);
            bo_line_kv(&l, "t_ms", bo_time_now_ns() / 1000000);
            bo_line_end(&l);
        }
        if (!serve_control(0)) { bo_posix_close(fd); return 0; }
    }
    bo_posix_close(fd);
    /* Read back from the DISK, chunk by chunk, still answering heartbeats. */
    do {
        rc = bo_update_install_verify(cap, m->bytes);
        if (!serve_control(0)) return 0;
    } while (rc > 0);
    check("read back from the disk", rc == 0, rc);
    if (rc != 0) return 0;
    if (m->selftest) {
        /* HASH VALID is decided on what the disk returned: one flipped bit
         * in the expected digest and the kernel refuses the candidate. */
        bo_u8 faux[32];
        int k;
        for (k = 0; k < 32; k++) faux[k] = m->sha256[k];
        faux[31] ^= 1;
        rc = bo_update_install_finish(cap, faux, m->bytes, (bo_u32)m->attempts);
        check("a digest other than the signed one is refused", rc == -BO_E_ACCESS_DENIED, rc);
    }
    rc = bo_update_install_finish(cap, m->sha256, m->bytes, (bo_u32)m->attempts);
    check("finish: read back, SHA-256 matches, candidate declared", rc == 0, rc);
    bo_line_begin(&l);
    bo_line_str(&l, rc == 0 ? "UPDATED_INSTALL_OK" : "UPDATED_INSTALL_FAIL");
    bo_line_kv(&l, "target", target);
    bo_line_kv(&l, "bytes", (bo_i64)m->bytes);
    bo_line_kv(&l, "attempts", (bo_i64)m->attempts);
    bo_line_str(&l, rc == 0 ? " signature=trusted hash=valid" : " signature=? hash=?");
    bo_line_end(&l);
    return rc == 0;
}

int bo_main(const struct bo_start *start)
{
    struct bo_handle_info info;
    struct bo_update_state st;
    struct manifest m;
    bo_i64 w, rc;
    int i;

    if (start->bootstrap_count != 2) { bo_say("[UPDATED] FAIL bootstrap_count\n"); return 2; }
    control = start->bootstrap[0];
    cap = start->bootstrap[1];
    if (bo_handle_info(cap, &info) != 0 || info.kind != BO_KIND_UPDATE) {
        bo_say("[UPDATED] FAIL no_update_capability\n");
        return 2;
    }
    w = bo_waitset_create();
    if (w < 0 || bo_waitset_add((bo_handle_t)w, control, KEY_CONTROL) != 0) return 3;
    ws = (bo_handle_t)w;
    /* Answer hello first: the supervisor starts its clock at spawn. */
    for (i = 0; i < 50; i++) serve_control(10000000UL);

    for (i = 0; i < 32; i++) m.image[i] = 0;
    m.bytes = m.attempts = 0; m.selftest = m.has_sha = 0;
    rc = parse_manifest(&m);
    {
        struct bo_line l;
        bo_line_begin(&l);
        bo_line_str(&l, "UPDATED_BEGIN");
        bo_line_kv(&l, "rights", (bo_i64)info.rights);
        bo_line_kv(&l, "manifest", rc);
        bo_line_end(&l);
    }
    if (rc == 1 && bo_update_state(cap, &st) == 0) {
        int running = (int)st.booted_slot, target = 1 - running;
        if (m.selftest) selftest_refusals(&m, running, target);
        if (st.state_a == 1 || st.state_b == 1) {
            bo_say("UPDATED_SKIP candidate_already_pending\n");
        } else if (st.active_slot != st.booted_slot) {
            bo_say("UPDATED_SKIP not_running_the_active_slot\n");
        } else {
            install(&m, target);
        }
        if (m.selftest) {
            struct bo_line l;
            bo_line_begin(&l);
            bo_line_str(&l, failures ? "UPDATED_REFUSALS_FAIL" : "UPDATED_REFUSALS_OK");
            bo_line_kv(&l, "failures", failures);
            bo_line_end(&l);
        }
    } else if (rc == 0) {
        bo_say("UPDATED_IDLE no_pending_update\n");
    } else {
        bo_say("UPDATED_IDLE manifest_invalid\n");
    }
    for (;;) if (!serve_control(1000000000UL)) return 0;
}
