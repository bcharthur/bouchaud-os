/*
 * serviced -- Bouchaud native service supervisor (ring 3).
 *
 * Policy lives HERE, in user space. The kernel only provides mechanisms:
 * process spawn with attenuated bootstrap handles, channels, process objects
 * whose TERMINATED signal rises at death, wait sets, time.
 *
 * Per-service state machine (every transition is logged on one line):
 *
 *      Stopped --start--> Starting --"ready"--> Running
 *      Running --missed heartbeats--> Degraded --kill--> (death)
 *      Running/Starting/Degraded --death--> Restarting   (budget left)
 *                                      \---> Failed       (budget spent)
 *      Restarting --backoff elapsed--> Starting
 *      Running --stop--> Stopping --death--> Stopped
 *
 * Restart policy: exponential backoff (BACKOFF_MIN * 2^n, capped), and a
 * crash budget: more than BUDGET_RESTARTS deaths within BUDGET_WINDOW and
 * the service is Failed -- no restart storm. A dependency is started only
 * once the service it depends on is Running.
 *
 * Usage:
 *   serviced            supervise the built-in graph forever
 *   serviced qualify    run the qualification scenario and exit 0/1
 */
#include "bouchaud/sdk.h"

#define MAX_SERVICES      4
#define KEY_CHAN(i)       (100u + (bo_u64)(i))
#define KEY_PROC(i)       (200u + (bo_u64)(i))

#define MS                1000000UL
#define BACKOFF_MIN       (50 * MS)
#define BACKOFF_MAX       (2000 * MS)
#define BUDGET_RESTARTS   3
#define BUDGET_WINDOW     (60000 * MS)
#define HEARTBEAT_PERIOD  (250 * MS)
#define HEARTBEAT_MISSES  4
#define STOP_GRACE        (2000 * MS)

enum state { STOPPED, STARTING, RUNNING, DEGRADED, RESTARTING, STOPPING, FAILED };

static const char *state_name(enum state s)
{
    switch (s) {
    case STOPPED:    return "Stopped";
    case STARTING:   return "Starting";
    case RUNNING:    return "Running";
    case DEGRADED:   return "Degraded";
    case RESTARTING: return "Restarting";
    case STOPPING:   return "Stopping";
    case FAILED:     return "Failed";
    }
    return "?";
}

struct service {
    const char  *name;
    const char  *path;
    int          depends_on;          /* index, or -1 */
    enum state   state;
    bo_handle_t  chan;                /* supervisor end of the control channel */
    bo_handle_t  proc;                /* process handle */
    bo_u64       pid;
    bo_u64       deaths[BUDGET_RESTARTS + 1];
    int          death_count;         /* total, for the budget ring */
    bo_u64       restarts;
    bo_u64       backoff;
    bo_u64       restart_at;
    bo_u64       stop_deadline;
    bo_u64       ping_seq;
    bo_u64       ping_sent_at;        /* 0: no ping in flight */
    bo_u64       last_pong_at;
    int          killed;
    bo_u64       last_rtt;
    int          missed;
    int          last_exit;
    bo_u64       spawned_at;
    bo_u64       running_at;
    int          want_running;
};

static struct service svc[MAX_SERVICES];
static int svc_count;
static bo_handle_t ws;

/* ---- tiny formatting ---------------------------------------------------- */

static void kv(const char *k, bo_i64 v) { bo_puts(" "); bo_puts(k); bo_puts("="); bo_put_i64(v); }

static bo_u64 now(void) { bo_i64 t = bo_time_now_ns(); return t < 0 ? 0 : (bo_u64)t; }

static int prefix(const char *s, bo_usize n, const char *p)
{
    bo_usize i = 0;
    while (p[i]) { if (i >= n || s[i] != p[i]) return 0; i++; }
    return 1;
}

static bo_u64 parse_u64(const char *s, bo_usize n, bo_usize *at)
{
    bo_u64 v = 0;
    while (*at < n && s[*at] == ' ') (*at)++;
    while (*at < n && s[*at] >= '0' && s[*at] <= '9') v = v * 10 + (bo_u64)(s[(*at)++] - '0');
    return v;
}

static bo_usize put_str(char *d, bo_usize at, bo_usize cap, const char *s)
{
    while (*s && at < cap) d[at++] = *s++;
    return at;
}

static bo_usize put_u64(char *d, bo_usize at, bo_usize cap, bo_u64 v)
{
    char t[24]; int i = 24;
    if (!v) t[--i] = '0';
    while (v) { t[--i] = (char)('0' + v % 10); v /= 10; }
    while (i < 24 && at < cap) d[at++] = t[i++];
    return at;
}

/* ---- state machine ------------------------------------------------------ */

static void transition(struct service *s, enum state to, const char *why)
{
    bo_puts("[SERVICED] svc=");
    bo_puts(s->name);
    bo_puts(" ");
    bo_puts(state_name(s->state));
    bo_puts("->");
    bo_puts(state_name(to));
    bo_puts(" why=");
    bo_puts(why);
    kv("pid", (bo_i64)s->pid);
    kv("t_ms", (bo_i64)(now() / MS));
    bo_puts("\n");
    s->state = to;
}

static bo_i64 send_line(struct service *s, const char *text)
{
    return bo_channel_send(s->chan, text, bo_strlen(text), 0, 0);
}

static void release(struct service *s)
{
    if (s->chan) {
        bo_waitset_remove(ws, KEY_CHAN(s - svc));
        bo_handle_close(s->chan);
        s->chan = 0;
    }
    if (s->proc) {
        bo_waitset_remove(ws, KEY_PROC(s - svc));
        bo_handle_close(s->proc);
        s->proc = 0;
    }
}

static int start(struct service *s)
{
    bo_handle_t pair[2];
    bo_handle_t give[1];
    bo_u32 mask[1];
    struct bo_spawn_result r;
    char hello[64];
    bo_usize n;
    bo_i64 rc;

    if (bo_channel_create(pair) != 0) return -1;
    give[0] = pair[1];
    /* DENY by default: the service may read, write and wait on its control
     * channel. It may not duplicate or pass it on. */
    mask[0] = BO_RIGHT_READ | BO_RIGHT_WRITE | BO_RIGHT_WAIT;
    s->spawned_at = now();
    rc = bo_process_spawn(s->path, bo_strlen(s->path), give, 1, mask, &r);
    /* Our copy of the child's end must go, or the channel would outlive the
     * child and PEER_CLOSED would never rise. */
    bo_handle_close(pair[1]);
    if (rc != 0) {
        bo_handle_close(pair[0]);
        bo_puts("[SERVICED] svc=");
        bo_puts(s->name);
        bo_puts(" spawn_error");
        kv("rc", rc);
        bo_puts("\n");
        return -1;
    }
    s->chan = pair[0];
    s->proc = r.process;
    s->pid = r.pid;
    s->ping_sent_at = 0;
    s->missed = 0;
    s->killed = 0;
    bo_waitset_add(ws, s->chan, KEY_CHAN(s - svc));
    bo_waitset_add(ws, s->proc, KEY_PROC(s - svc));
    transition(s, STARTING, "spawn");
    n = put_str(hello, 0, sizeof(hello), "hello ");
    n = put_str(hello, n, sizeof(hello), s->name);
    bo_channel_send(s->chan, hello, n, 0, 0);
    return 0;
}

static int budget_spent(struct service *s, bo_u64 t)
{
    int i, recent = 0;
    s->deaths[s->death_count % (BUDGET_RESTARTS + 1)] = t;
    s->death_count++;
    for (i = 0; i < BUDGET_RESTARTS + 1 && i < s->death_count; i++) {
        if (t - s->deaths[i] <= BUDGET_WINDOW) recent++;
    }
    return recent > BUDGET_RESTARTS;
}

static void on_death(struct service *s)
{
    struct bo_process_info info;
    bo_u64 t = now();
    bo_i64 sig_chan;
    struct bo_handle_info hi;

    info.exit_code = -1;
    bo_process_info(s->proc, &info);
    s->last_exit = info.exit_code;

    /* Platform invariant: when TERMINATED is visible, the dead process'
     * handles are already released, so its channel shows PEER_CLOSED. */
    sig_chan = bo_handle_info(s->chan, &hi);
    bo_puts("[SERVICED] svc=");
    bo_puts(s->name);
    bo_puts(" death");
    kv("pid", (bo_i64)s->pid);
    kv("exit", info.exit_code);
    kv("peer_closed", sig_chan == 0 && (hi.signals & BO_SIGNAL_PEER_CLOSED) ? 1 : 0);
    bo_puts("\n");
    if (!(sig_chan == 0 && (hi.signals & BO_SIGNAL_PEER_CLOSED))) {
        bo_puts("[SERVICED] INVARIANT_VIOLATION terminated_without_peer_closed svc=");
        bo_puts(s->name);
        bo_puts("\n");
    }
    release(s);

    if (s->state == STOPPING || !s->want_running) {
        transition(s, STOPPED, info.exit_code == 0 ? "stopped" : "stopped_by_kill");
        return;
    }
    if (budget_spent(s, t)) {
        transition(s, FAILED, "crash_budget_spent");
        return;
    }
    s->backoff = s->backoff ? s->backoff * 2 : BACKOFF_MIN;
    if (s->backoff > BACKOFF_MAX) s->backoff = BACKOFF_MAX;
    s->restart_at = t + s->backoff;
    s->restarts++;
    transition(s, RESTARTING, s->killed ? "killed_unresponsive"
                              : info.exit_code == 0 ? "exit" : "crash");
}

static void on_message(struct service *s)
{
    char in[160];
    struct bo_recv_meta meta;
    bo_i64 got;

    for (;;) {
        got = bo_channel_recv(s->chan, in, sizeof(in), 0, 0, &meta);
        if (got < 0) return;               /* would block, or peer closed */
        if (prefix(in, (bo_usize)got, "ready ") && s->state == STARTING) {
            s->running_at = now();
            s->last_pong_at = s->running_at;
            transition(s, RUNNING, "ready");
            bo_puts("[SERVICED] svc=");
            bo_puts(s->name);
            bo_puts(" start_latency");
            kv("ms", (bo_i64)((s->running_at - s->spawned_at) / MS));
            bo_puts("\n");
            /* A clean start resets the backoff, not the crash budget. */
            s->backoff = 0;
        } else if (prefix(in, (bo_usize)got, "pong ")) {
            bo_usize at = 5;
            bo_u64 seq = parse_u64(in, (bo_usize)got, &at);
            if (seq == s->ping_seq && s->ping_sent_at) {
                s->last_pong_at = now();
                s->last_rtt = s->last_pong_at - s->ping_sent_at;
                s->ping_sent_at = 0;
                s->missed = 0;
                if (s->state == DEGRADED) transition(s, RUNNING, "heartbeat_back");
            }
        }
    }
}

/* One ping per period. A ping unanswered for a whole period is a miss; the
 * next ping replaces it. HEARTBEAT_MISSES consecutive misses: the service is
 * hung, not slow -> Degraded, then killed once (its death drives recovery). */
static void heartbeat(struct service *s, bo_u64 t)
{
    char line[48];
    bo_usize n;
    if (s->state != RUNNING && s->state != DEGRADED) return;
    if (s->killed) return;
    if (s->ping_sent_at) {
        if (t - s->ping_sent_at < HEARTBEAT_PERIOD) return;
        s->missed++;
        if (s->missed >= HEARTBEAT_MISSES) {
            if (s->state == RUNNING) transition(s, DEGRADED, "heartbeat_missed");
            bo_process_kill(s->proc);
            s->killed = 1;
            return;
        }
    } else if (t - s->last_pong_at < HEARTBEAT_PERIOD) {
        return;
    }
    s->ping_seq++;
    n = put_str(line, 0, sizeof(line), "ping ");
    n = put_u64(line, n, sizeof(line), s->ping_seq);
    s->ping_sent_at = t;
    bo_channel_send(s->chan, line, n, 0, 0);
}

/* One supervisor step: wait at most `budget` ns, then run due work. */
static void step(bo_u64 budget)
{
    struct bo_wait_event ev[2 * MAX_SERVICES];
    bo_i64 n, i;
    bo_u64 t;
    int k;

    n = bo_waitset_wait(ws, ev, 2 * MAX_SERVICES, budget);
    for (i = 0; i < n; i++) {
        bo_u64 key = ev[i].key;
        if (key >= 200 && key < 200 + MAX_SERVICES) {
            struct service *s = &svc[key - 200];
            if (s->proc && (ev[i].signals & BO_SIGNAL_TERMINATED)) {
                on_message(s);             /* drain last words first */
                on_death(s);
            }
        } else if (key >= 100 && key < 100 + MAX_SERVICES) {
            struct service *s = &svc[key - 100];
            if (s->chan && (ev[i].signals & BO_SIGNAL_READABLE)) on_message(s);
        }
    }
    t = now();
    for (k = 0; k < svc_count; k++) {
        struct service *s = &svc[k];
        if (s->state == RESTARTING && t >= s->restart_at) start(s);
        if (s->state == STOPPED && s->want_running
            && (s->depends_on < 0 || svc[s->depends_on].state == RUNNING))
            start(s);
        if (s->state == STOPPING && t >= s->stop_deadline && s->proc) {
            bo_process_kill(s->proc);
            s->stop_deadline = t + STOP_GRACE;
        }
        heartbeat(s, t);
    }
}

static void stop(struct service *s)
{
    s->want_running = 0;
    if (s->state == RUNNING || s->state == DEGRADED || s->state == STARTING) {
        send_line(s, "stop");
        s->stop_deadline = now() + STOP_GRACE;
        transition(s, STOPPING, "stop_requested");
    } else if (s->state == RESTARTING || s->state == FAILED) {
        transition(s, STOPPED, "stop_requested");
    }
}

static void add_service(const char *name, const char *path, int depends_on)
{
    struct service *s = &svc[svc_count++];
    s->name = name;
    s->path = path;
    s->depends_on = depends_on;
    s->state = STOPPED;
    s->want_running = 1;
}

/* ---- qualification scenario --------------------------------------------- */

static int failures;

static void verdict(const char *what, int ok)
{
    bo_puts(ok ? "[SERVICED] ok   " : "[SERVICED] FAIL ");
    bo_puts(what);
    bo_puts("\n");
    if (!ok) failures++;
}

/* Run the loop until `s` reaches `want` or `limit` ns elapse. Failed is
 * terminal: waiting for any other state once there would only burn the
 * whole limit (and did, 10 s, in the first TCG run of this scenario). */
static int run_until(struct service *s, enum state want, bo_u64 limit)
{
    bo_u64 end = now() + limit;
    while (s->state != want) {
        if (s->state == FAILED) return 0;
        bo_u64 t = now();
        if (t >= end) return 0;
        step(end - t < 20 * MS ? end - t : 20 * MS);
    }
    return 1;
}

static void sort_u64(bo_u64 *a, int n)
{
    int i, j;
    for (i = 1; i < n; i++) {
        bo_u64 v = a[i];
        for (j = i - 1; j >= 0 && a[j] > v; j--) a[j + 1] = a[j];
        a[j + 1] = v;
    }
}

#define RTT_SAMPLES 200
static bo_u64 rtt[RTT_SAMPLES];

static int measure_ipc(struct service *s)
{
    int i;
    struct bo_wait_event ev[1];
    bo_i64 single;
    char line[48], in[160];
    struct bo_recv_meta meta;

    /* A private wait set on the control channel alone: the measurement is
     * the round trip, not the supervisor's scheduling of other services. */
    single = bo_waitset_create();
    if (single < 0 || bo_waitset_add((bo_handle_t)single, s->chan, 1) != 0) return 0;
    for (i = 0; i < RTT_SAMPLES; i++) {
        bo_usize n = put_str(line, 0, sizeof(line), "ping ");
        bo_u64 t0;
        n = put_u64(line, n, sizeof(line), 1000000 + (bo_u64)i);
        t0 = now();
        if (bo_channel_send(s->chan, line, n, 0, 0) < 0) return 0;
        for (;;) {
            bo_i64 got;
            if (bo_waitset_wait((bo_handle_t)single, ev, 1, 2000 * MS) <= 0) return 0;
            got = bo_channel_recv(s->chan, in, sizeof(in), 0, 0, &meta);
            if (got > 0 && prefix(in, (bo_usize)got, "pong ")) {
                bo_usize at = 5;
                /* A late heartbeat pong is not this round trip. */
                if (parse_u64(in, (bo_usize)got, &at) == 1000000 + (bo_u64)i) break;
            }
            if (got == -BO_E_PEER_CLOSED) return 0;
        }
        rtt[i] = now() - t0;
    }
    bo_handle_close((bo_handle_t)single);
    s->ping_sent_at = 0;
    s->missed = 0;
    s->last_pong_at = now();
    sort_u64(rtt, RTT_SAMPLES);
    bo_puts("SERVICED_IPC_RTT_NS");
    kv("n", RTT_SAMPLES);
    kv("p50", (bo_i64)rtt[RTT_SAMPLES / 2]);
    kv("p95", (bo_i64)rtt[RTT_SAMPLES * 95 / 100]);
    kv("p99", (bo_i64)rtt[RTT_SAMPLES * 99 / 100]);
    kv("max", (bo_i64)rtt[RTT_SAMPLES - 1]);
    bo_puts("\n");
    return 1;
}

static int qualify(void)
{
    struct service *health, *echo;
    bo_i64 baseline, steady, after;
    bo_u64 t0;
    int i;

    bo_puts("[SERVICED] QUALIFY_BEGIN\n");
    baseline = bo_handle_count();

    add_service("health", "/bin/healthd", -1);
    add_service("echo", "/bin/healthd", 0);
    health = &svc[0];
    echo = &svc[1];

    /* 1. dependency order: echo must not start before health is Running. */
    verdict("health reaches Running", run_until(health, RUNNING, 20000 * MS));
    verdict("echo was not started before health was Running",
            echo->spawned_at == 0 || echo->spawned_at >= health->running_at);
    verdict("echo reaches Running", run_until(echo, RUNNING, 20000 * MS));
    steady = bo_handle_count();

    /* 2. IPC round-trip distribution through a real native channel. */
    verdict("ipc rtt measured (200 round trips)", measure_ipc(health));

    /* 3. crash injection: user fault -> containment -> detection -> restart. */
    t0 = now();
    send_line(health, "crash");
    verdict("crashed service detected and restarting",
            run_until(health, RESTARTING, 10000 * MS));
    verdict("crash exit code is a fault, not a clean exit", health->last_exit != 0);
    verdict("crashed service back to Running", run_until(health, RUNNING, 20000 * MS));
    bo_puts("SERVICED_RECOVERY_MS");
    kv("crash_to_running", (bo_i64)((health->running_at - t0) / MS));
    bo_puts("\n");
    verdict("dependent service untouched by the crash", echo->state == RUNNING && echo->restarts == 0);
    after = bo_handle_count();
    bo_puts("SERVICED_HANDLES");
    kv("baseline", baseline);
    kv("steady", steady);
    kv("after_restart", after);
    bo_puts("\n");
    verdict("no handle leak across a crash/restart", after == steady);

    /* 4. hang: heartbeat misses -> Degraded -> kill -> restart. */
    send_line(health, "hang");
    verdict("hung service detected (Degraded)", run_until(health, DEGRADED, 20000 * MS));
    verdict("hung service killed and restarting", run_until(health, RESTARTING, 20000 * MS));
    verdict("hung service back to Running", run_until(health, RUNNING, 20000 * MS));

    /* 5. crash budget: no restart storm. */
    for (i = 0; i < 3 && health->state != FAILED; i++) {
        send_line(health, "crash");
        if (!run_until(health, RESTARTING, 10000 * MS)) break;
        if (health->state == FAILED) break;
        run_until(health, RUNNING, 20000 * MS);
    }
    if (health->state != FAILED) run_until(health, FAILED, 5000 * MS);
    verdict("crash budget spent -> Failed (no restart storm)", health->state == FAILED);
    bo_puts("SERVICED_RESTARTS");
    kv("health", (bo_i64)health->restarts);
    kv("budget", BUDGET_RESTARTS);
    bo_puts("\n");

    /* 6. orderly shutdown. */
    stop(echo);
    stop(health);
    verdict("echo stopped cleanly", run_until(echo, STOPPED, 10000 * MS) && echo->last_exit == 0);
    verdict("health stopped", health->state == STOPPED);
    after = bo_handle_count();
    bo_puts("SERVICED_HANDLES");
    kv("baseline", baseline);
    kv("final", after);
    bo_puts("\n");
    verdict("every handle released at shutdown", after == baseline);

    if (failures) {
        bo_puts("SERVICED_QUALIFY_FAIL");
        kv("failures", failures);
        bo_puts("\n");
        return 1;
    }
    bo_puts("SERVICED_QUALIFY_OK\n");
    return 0;
}

int bo_main(const struct bo_start *start)
{
    bo_i64 w;
    bo_i64 v = bo_version();
    if (v < 0 || (v >> 16) != BO_ABI_MAJOR || (v & 0xffff) < BO_ABI_MINOR) {
        bo_puts("[SERVICED] FAIL abi_version\n");
        return 2;
    }
    w = bo_waitset_create();
    if (w < 0) return 3;
    ws = (bo_handle_t)w;

    if (start->argc > 1 && prefix(start->argv[1], bo_strlen(start->argv[1]), "qualify"))
        return qualify();

    add_service("health", "/bin/healthd", -1);
    for (;;) step(HEARTBEAT_PERIOD);
}
