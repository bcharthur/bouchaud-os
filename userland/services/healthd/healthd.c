/*
 * healthd -- the first supervised native ring 3 service of Bouchaud OS.
 *
 * Deliberately small: its purpose is to exercise the platform contract, not
 * to be clever. It owns NO ambient authority. Everything it can touch comes
 * from ONE bootstrap handle: the control channel its supervisor created and
 * passed at spawn, attenuated to READ|WRITE|WAIT (no DUP, no TRANSFER).
 *
 * Protocol (one message = one ASCII line, no handles):
 *
 *   supervisor -> healthd           healthd -> supervisor
 *   "hello <name>"                  "ready <name> <pid-agnostic>"
 *   "ping <seq>"                    "pong <seq> <handles> <t_rx_ns>"
 *   "crash"                         (null dereference: fault injection)
 *   "hang"                          (stops answering: heartbeat test)
 *   "stop"                          "bye", then exit(0)
 *
 * A closed control channel (supervisor gone) is an orderly shutdown: exit(0).
 * healthd never busy-waits: it blocks in bo_waitset_wait.
 */
#include "bouchaud/sdk.h"

#define KEY_CONTROL 1

static int prefix(const char *s, bo_usize n, const char *p)
{
    bo_usize i = 0;
    while (p[i]) {
        if (i >= n || s[i] != p[i]) return 0;
        i++;
    }
    return 1;
}

static bo_usize append(char *dst, bo_usize at, bo_usize cap, const char *src, bo_usize n)
{
    bo_usize i;
    for (i = 0; i < n && at < cap; i++) dst[at++] = src[i];
    return at;
}

static bo_usize append_u64(char *dst, bo_usize at, bo_usize cap, bo_u64 v)
{
    char tmp[24];
    int i = 24;
    if (v == 0) tmp[--i] = '0';
    while (v) { tmp[--i] = (char)('0' + v % 10); v /= 10; }
    return append(dst, at, cap, tmp + i, (bo_usize)(24 - i));
}

int bo_main(const struct bo_start *start)
{
    bo_handle_t control;
    bo_i64 ws;
    char in[128];
    char out[160];
    struct bo_recv_meta meta;
    struct bo_wait_event ev[2];

    if (start->bootstrap_count != 1) {
        bo_say("[HEALTHD] FAIL bootstrap_count\n");
        return 2;
    }
    control = start->bootstrap[0];

    ws = bo_waitset_create();
    if (ws < 0 || bo_waitset_add((bo_handle_t)ws, control, KEY_CONTROL) != 0) {
        bo_say("[HEALTHD] FAIL waitset\n");
        return 3;
    }

    for (;;) {
        bo_i64 n = bo_waitset_wait((bo_handle_t)ws, ev, 2, 5000000000UL);
        bo_i64 got;
        bo_usize len;

        if (n == -BO_E_TIMED_OUT || n == 0) continue;
        if (n < 0) return 4;

        got = bo_channel_recv(control, in, sizeof(in), 0, 0, &meta);
        if (got == -BO_E_PEER_CLOSED) return 0;         /* supervisor gone */
        if (got == -BO_E_WOULD_BLOCK) continue;
        if (got < 0) return 5;

        len = 0;
        if (prefix(in, (bo_usize)got, "ping ")) {
            len = append(out, len, sizeof(out), "pong ", 5);
            len = append(out, len, sizeof(out), in + 5, (bo_usize)got - 5);
            len = append(out, len, sizeof(out), " ", 1);
            len = append_u64(out, len, sizeof(out), (bo_u64)bo_handle_count());
            /* Reception time: lets the supervisor split a round trip into
             * its forward (healthd woken) and return (supervisor woken) legs. */
            len = append(out, len, sizeof(out), " ", 1);
            len = append_u64(out, len, sizeof(out), (bo_u64)bo_time_now_ns());
        } else if (prefix(in, (bo_usize)got, "hello ")) {
            len = append(out, len, sizeof(out), "ready ", 6);
            len = append(out, len, sizeof(out), in + 6, (bo_usize)got - 6);
        } else if (prefix(in, (bo_usize)got, "crash")) {
            /* Fault injection: a real user-mode fault, not an exit code. */
            volatile int *nowhere = (volatile int *)0;
            *nowhere = 0x0b0b;
            return 6;
        } else if (prefix(in, (bo_usize)got, "hang")) {
            /* Fault injection: alive but unresponsive (heartbeat test). */
            for (;;) bo_sleep_ns(1000000000UL);
        } else if (prefix(in, (bo_usize)got, "stop")) {
            (void)bo_channel_send(control, "bye", 3, 0, 0);
            return 0;
        } else {
            len = append(out, len, sizeof(out), "error", 5);
        }
        if (bo_channel_send(control, out, len, 0, 0) < 0) return 0;
    }
}
