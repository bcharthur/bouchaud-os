/*
 * logd -- structured system log, ring 3 (BOUCHAUD_LOGD_V1).
 *
 * Supervised by serviced like any service. Bootstrap handles:
 *   [0] control channel   READ | WRITE | WAIT   (hello/ping/stop/stats)
 *   [1] intake channel    READ | WAIT           (events, from serviced)
 *
 * An event is ONE message, one ASCII line of key=value fields:
 *   "sev=<0..7> svc=<name> pid=<n> type=<word> [k=v ...]"
 * logd stamps each event with its reception time (ts_ns) and the boot id it
 * was given ("boot <id>" on the control channel), keeps it in a BOUNDED ring
 * (EVENTS_MAX) and writes it to the console as
 *   LOGD ts_ns=.. boot=.. sev=.. svc=.. pid=.. type=.. k=v...
 *
 * Bounds and backpressure -- nothing ever blocks:
 *   * the kernel never sends to logd; logd never waits on the kernel beyond
 *     its own wait set;
 *   * a producer sends without blocking: a full intake queue is
 *     -BO_E_QUEUE_FULL, which the PRODUCER counts and reports in its next
 *     accepted event (dropped=<n>): a loss is never silent;
 *   * logd's ring overwrites its oldest event when full and counts it
 *     (overwritten); "stats" returns received/overwritten/malformed.
 */
#include "bouchaud/sdk.h"

#define KEY_CONTROL 1
#define KEY_INTAKE  2
#define EVENTS_MAX  256
#define EVENT_CAP   200

static char ring[EVENTS_MAX][EVENT_CAP];
static bo_u64 received, overwritten, malformed, boot_id;
static int head;

static int prefix(const char *s, bo_usize n, const char *p)
{
    bo_usize i = 0;
    while (p[i]) { if (i >= n || s[i] != p[i]) return 0; i++; }
    return 1;
}

static bo_usize put(char *d, bo_usize at, bo_usize cap, const char *s, bo_usize n)
{
    bo_usize i;
    for (i = 0; i < n && at < cap; i++) d[at++] = s[i];
    return at;
}

static bo_usize put_u64(char *d, bo_usize at, bo_usize cap, bo_u64 v)
{
    char t[24]; int i = 24;
    if (!v) t[--i] = '0';
    while (v) { t[--i] = (char)('0' + v % 10); v /= 10; }
    return put(d, at, cap, t + i, (bo_usize)(24 - i));
}

/* A well-formed event starts with sev=<digit> and holds svc= and type=. */
static int valid(const char *s, bo_usize n)
{
    bo_usize i;
    int svc = 0, type = 0;
    if (n < 6 || !prefix(s, n, "sev=") || s[4] < '0' || s[4] > '7') return 0;
    for (i = 0; i < n; i++) {
        if (s[i] == '\n' || s[i] == 0) return 0;
        if (i + 4 <= n && (i == 0 || s[i - 1] == ' ') && prefix(s + i, n - i, "svc=")) svc = 1;
        if (i + 5 <= n && (i == 0 || s[i - 1] == ' ') && prefix(s + i, n - i, "type=")) type = 1;
    }
    return svc && type;
}

static void ingest(const char *s, bo_usize n)
{
    struct bo_line l;
    char *slot;
    bo_usize at;
    if (!valid(s, n)) { malformed++; return; }
    received++;
    if (received > EVENTS_MAX) overwritten++;
    slot = ring[head];
    head = (head + 1) % EVENTS_MAX;
    at = put(slot, 0, EVENT_CAP - 1, s, n);
    slot[at] = 0;
    bo_line_begin(&l);
    bo_line_str(&l, "LOGD");
    bo_line_kv(&l, "ts_ns", bo_time_now_ns());
    bo_line_kv(&l, "boot", (bo_i64)boot_id);
    bo_line_str(&l, " ");
    bo_line_str(&l, slot);
    bo_line_end(&l);
}

int bo_main(const struct bo_start *start)
{
    bo_handle_t control, intake;
    bo_i64 ws;
    struct bo_wait_event ev[2];
    struct bo_recv_meta meta;
    char in[EVENT_CAP], out[160];

    if (start->bootstrap_count != 2) { bo_say("[LOGD] FAIL bootstrap_count\n"); return 2; }
    control = start->bootstrap[0];
    intake = start->bootstrap[1];
    ws = bo_waitset_create();
    if (ws < 0 || bo_waitset_add((bo_handle_t)ws, control, KEY_CONTROL) != 0
        || bo_waitset_add((bo_handle_t)ws, intake, KEY_INTAKE) != 0) return 3;

    for (;;) {
        bo_i64 n = bo_waitset_wait((bo_handle_t)ws, ev, 2, 5000000000UL), i;
        if (n == -BO_E_TIMED_OUT || n == 0) continue;
        if (n < 0) return 4;
        for (i = 0; i < n; i++) {
            if (ev[i].key == KEY_INTAKE) {
                /* Drain what is there, bounded per round so control is
                 * never starved by a flood. */
                int k;
                for (k = 0; k < 64; k++) {
                    bo_i64 got = bo_channel_recv(intake, in, sizeof(in), 0, 0, &meta);
                    if (got < 0) break;
                    ingest(in, (bo_usize)got);
                }
                continue;
            }
            {
                bo_i64 got = bo_channel_recv(control, in, sizeof(in), 0, 0, &meta);
                bo_usize len = 0;
                if (got == -BO_E_PEER_CLOSED) return 0;
                if (got < 0) continue;
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
                } else if (prefix(in, (bo_usize)got, "boot ")) {
                    bo_usize at = 5; bo_u64 v = 0;
                    while (at < (bo_usize)got && in[at] >= '0' && in[at] <= '9') v = v * 10 + (bo_u64)(in[at++] - '0');
                    boot_id = v;
                    len = put(out, 0, sizeof(out), "ok", 2);
                } else if (prefix(in, (bo_usize)got, "stats")) {
                    /* Drain first: stats describe everything sent before. */
                    for (;;) {
                        bo_i64 g = bo_channel_recv(intake, in, sizeof(in), 0, 0, &meta);
                        if (g < 0) break;
                        ingest(in, (bo_usize)g);
                    }
                    len = put(out, 0, sizeof(out), "stats received=", 15);
                    len = put_u64(out, len, sizeof(out), received);
                    len = put(out, len, sizeof(out), " overwritten=", 13);
                    len = put_u64(out, len, sizeof(out), overwritten);
                    len = put(out, len, sizeof(out), " malformed=", 11);
                    len = put_u64(out, len, sizeof(out), malformed);
                    len = put(out, len, sizeof(out), " ring=", 6);
                    len = put_u64(out, len, sizeof(out), EVENTS_MAX);
                } else if (prefix(in, (bo_usize)got, "stop")) {
                    (void)bo_channel_send(control, "bye", 3, 0, 0);
                    return 0;
                } else {
                    len = put(out, 0, sizeof(out), "error", 5);
                }
                bo_channel_send(control, out, len, 0, 0);
            }
        }
    }
}
