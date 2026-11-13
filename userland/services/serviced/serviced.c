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
 *   serviced system     THE system supervisor (BOUCHAUD_SERVICED_SYSTEM_V1),
 *                       started by the kernel with one bootstrap handle: the
 *                       update capability (INSPECT | UPDATE_HEALTH).
 *
 * System state machine (one line per transition, "[SERVICED] system A->B"):
 *
 *   Booting --capability checked--> StartingServices
 *   StartingServices --every critical service Running for HEALTH_WINDOW--> Healthy
 *   Healthy --critical service leaves Running--> Degraded --back--> Healthy
 *   StartingServices/Degraded --critical service Failed, or no health
 *       within HEALTH_DEADLINE--> RecoveryRequired
 *
 * Entering Healthy for the first time declares the boot healthy through the
 * capability (bo_update_commit_health): a candidate slot becomes the active,
 * Good slot. RecoveryRequired never commits: the loader keeps counting the
 * candidate's attempts and rolls it back. The critical list is versioned
 * (CRITICAL_PROFILE); changing it is a change of the health contract.
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
    int          critical;            /* part of the system health contract */
    bo_handle_t  extra;               /* second bootstrap handle, or 0 */
    bo_u32       extra_mask;          /* rights it crosses the spawn with */
    int          intake;              /* logd: a fresh intake channel per start */
};

static struct service svc[MAX_SERVICES];
static int svc_count;
static bo_handle_t ws;

/* ---- structured events to logd (BOUCHAUD_LOGD_V1) ------------------------ */
/* Never blocks: a full intake queue is counted here and reported in the next
 * accepted event as dropped=<n>. No logd (yet, or dead): counted the same. */
static bo_handle_t logd_intake;
static bo_u64 logd_boot_id;          /* handed to logd in its hello */
static bo_u64 log_sent, log_dropped, log_dropped_pending, log_calls;

static bo_i64 log_event(int sev, const char *svc_name, bo_u64 pid, const char *type, const char *fields)
{
    char m[200];
    bo_usize n = 0;
    bo_i64 rc;
    const char *p;
    #define PUT(str) for (p = (str); *p && n < sizeof(m); ) m[n++] = *p++
    PUT("sev="); if (n < sizeof(m)) m[n++] = (char)('0' + (sev & 7));
    PUT(" svc="); PUT(svc_name);
    PUT(" pid="); { char t[24]; int i = 24; bo_u64 v = pid; t[--i] = 0; if (!v) t[--i] = '0';
                    while (v) { t[--i] = (char)('0' + v % 10); v /= 10; } PUT(t + i); }
    PUT(" type="); PUT(type);
    if (fields && *fields) { PUT(" "); PUT(fields); }
    if (log_dropped_pending) {
        char t[24]; int i = 24; bo_u64 v = log_dropped_pending;
        while (v) { t[--i] = (char)('0' + v % 10); v /= 10; }
        PUT(" dropped="); { int k; for (k = i; k < 24 && n < sizeof(m); k++) m[n++] = t[k]; }
    }
    #undef PUT
    log_calls++;
    rc = logd_intake ? bo_channel_send(logd_intake, m, n, 0, 0) : -BO_E_BAD_HANDLE;
    if (rc >= 0) { log_sent++; log_dropped_pending = 0; }
    else { log_dropped++; log_dropped_pending++; }
    return rc;
}

/* ---- tiny formatting ---------------------------------------------------- */

static void kv(const char *k, bo_i64 v) { bo_say_kv(k, v); }

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
    bo_say("[SERVICED] svc=");
    bo_say(s->name);
    bo_say(" ");
    bo_say(state_name(s->state));
    bo_say("->");
    bo_say(state_name(to));
    bo_say(" why=");
    bo_say(why);
    kv("pid", (bo_i64)s->pid);
    kv("t_ms", (bo_i64)(now() / MS));
    bo_say("\n");
    {
        char f[96];
        bo_usize n = 0;
        const char *parts[6] = { "from=", state_name(s->state), " to=", state_name(to), " why=", why };
        int k;
        for (k = 0; k < 6; k++) { const char *p = parts[k]; while (*p && n + 1 < sizeof(f)) f[n++] = *p++; }
        f[n] = 0;
        log_event(to == FAILED || to == DEGRADED ? 3 : 6, s->name, s->pid, "transition", f);
    }
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
    bo_handle_t give[2];
    bo_u32 mask[2];
    struct bo_spawn_result r;
    char hello[64];
    bo_usize n;
    bo_i64 rc;

    if (bo_channel_create(pair) != 0) return -1;
    give[0] = pair[1];
    if (s->intake) {
        /* logd: a new intake channel per start. Its end crosses READ|WAIT
         * only; ours replaces the previous one (closed with the old logd). */
        bo_handle_t ip[2];
        if (bo_channel_create(ip) != 0) { bo_handle_close(pair[0]); bo_handle_close(pair[1]); return -1; }
        if (logd_intake) bo_handle_close(logd_intake);
        logd_intake = ip[0];
        s->extra = ip[1];
        s->extra_mask = BO_RIGHT_READ | BO_RIGHT_WAIT;
    }
    /* DENY by default: the service may read, write and wait on its control
     * channel. It may not duplicate or pass it on. */
    mask[0] = BO_RIGHT_READ | BO_RIGHT_WRITE | BO_RIGHT_WAIT;
    s->spawned_at = now();
    give[1] = s->extra;
    mask[1] = s->extra_mask;
    rc = bo_process_spawn(s->path, bo_strlen(s->path), give, s->extra ? 2 : 1, mask, &r);
    /* Our copy of the child's end must go, or the channel would outlive the
     * child and PEER_CLOSED would never rise. */
    bo_handle_close(pair[1]);
    if (s->intake && s->extra) { bo_handle_close(s->extra); s->extra = 0; }
    if (rc != 0) {
        bo_handle_close(pair[0]);
        bo_say("[SERVICED] svc=");
        bo_say(s->name);
        bo_say(" spawn_error");
        kv("rc", rc);
        bo_say("\n");
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
    if (s->intake && logd_boot_id) {
        n = put_str(hello, n, sizeof(hello), " boot=");
        n = put_u64(hello, n, sizeof(hello), logd_boot_id);
    }
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
    /* We are the POSIX parent: collect the zombie. The Process handle only
     * observed the death; it never collects (BOUCHAUD_NATIVE_PROCESS_OBSERVATEUR_V1).
     * TERMINATED is raised after the child became a zombie, so this returns
     * at once. */
    {
        int st = 0;
        bo_i64 r = bo_posix_wait4((bo_i64)s->pid, &st, 0);
        if (r != (bo_i64)s->pid) {
            bo_say("[SERVICED] INVARIANT_VIOLATION wait4_after_terminated svc=");
            bo_say(s->name);
            bo_say_kv("rc", r);
            bo_say("\n");
        }
    }

    /* Platform invariant: when TERMINATED is visible, the dead process'
     * handles are already released, so its channel shows PEER_CLOSED. */
    sig_chan = bo_handle_info(s->chan, &hi);
    bo_say("[SERVICED] svc=");
    bo_say(s->name);
    bo_say(" death");
    kv("pid", (bo_i64)s->pid);
    kv("exit", info.exit_code);
    kv("peer_closed", sig_chan == 0 && (hi.signals & BO_SIGNAL_PEER_CLOSED) ? 1 : 0);
    bo_say("\n");
    if (!(sig_chan == 0 && (hi.signals & BO_SIGNAL_PEER_CLOSED))) {
        bo_say("[SERVICED] INVARIANT_VIOLATION terminated_without_peer_closed svc=");
        bo_say(s->name);
        bo_say("\n");
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
            bo_say("[SERVICED] svc=");
            bo_say(s->name);
            bo_say(" start_latency");
            kv("ms", (bo_i64)((s->running_at - s->spawned_at) / MS));
            bo_say("\n");
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
    bo_say(ok ? "[SERVICED] ok   " : "[SERVICED] FAIL ");
    bo_say(what);
    bo_say("\n");
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
static bo_u64 aller[RTT_SAMPLES];
static bo_u64 retour[RTT_SAMPLES];

static void publie_distribution(const char *nom, bo_u64 *v)
{
    sort_u64(v, RTT_SAMPLES);
    bo_say(nom);
    kv("n", RTT_SAMPLES);
    kv("p50", (bo_i64)v[RTT_SAMPLES / 2]);
    kv("p95", (bo_i64)v[RTT_SAMPLES * 95 / 100]);
    kv("p99", (bo_i64)v[RTT_SAMPLES * 99 / 100]);
    kv("max", (bo_i64)v[RTT_SAMPLES - 1]);
    bo_say("\n");
}

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
                if (parse_u64(in, (bo_usize)got, &at) == 1000000 + (bo_u64)i) {
                    bo_u64 t1 = now();
                    bo_u64 rx;
                    (void)parse_u64(in, (bo_usize)got, &at);      /* handles */
                    rx = parse_u64(in, (bo_usize)got, &at);       /* t_rx    */
                    rtt[i] = t1 - t0;
                    aller[i] = rx > t0 ? rx - t0 : 0;
                    retour[i] = t1 > rx ? t1 - rx : 0;
                    break;
                }
            }
            if (got == -BO_E_PEER_CLOSED) return 0;
        }
    }
    bo_handle_close((bo_handle_t)single);
    s->ping_sent_at = 0;
    s->missed = 0;
    s->last_pong_at = now();
    publie_distribution("SERVICED_IPC_RTT_NS", rtt);
    publie_distribution("SERVICED_IPC_ALLER_NS", aller);
    publie_distribution("SERVICED_IPC_RETOUR_NS", retour);
    return 1;
}

static int qualify(void)
{
    struct service *health, *echo;
    bo_i64 baseline, steady, after;
    bo_u64 t0;
    int i;

    bo_say("[SERVICED] QUALIFY_BEGIN\n");
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
    bo_say("SERVICED_RECOVERY_MS");
    kv("crash_to_running", (bo_i64)((health->running_at - t0) / MS));
    bo_say("\n");
    verdict("dependent service untouched by the crash", echo->state == RUNNING && echo->restarts == 0);
    after = bo_handle_count();
    bo_say("SERVICED_HANDLES");
    kv("baseline", baseline);
    kv("steady", steady);
    kv("after_restart", after);
    bo_say("\n");
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
    bo_say("SERVICED_RESTARTS");
    kv("health", (bo_i64)health->restarts);
    kv("budget", BUDGET_RESTARTS);
    bo_say("\n");

    /* 6. orderly shutdown. */
    stop(echo);
    stop(health);
    verdict("echo stopped cleanly", run_until(echo, STOPPED, 10000 * MS) && echo->last_exit == 0);
    verdict("health stopped", health->state == STOPPED);
    after = bo_handle_count();
    bo_say("SERVICED_HANDLES");
    kv("baseline", baseline);
    kv("final", after);
    bo_say("\n");
    verdict("every handle released at shutdown", after == baseline);
    verdict("no zombie left (wait4(-1) = ECHILD)",
            bo_posix_wait4(-1, 0, BO_WAIT_NOHANG) == -10);

    if (failures) {
        bo_say("SERVICED_QUALIFY_FAIL");
        kv("failures", failures);
        bo_say("\n");
        return 1;
    }
    bo_say("SERVICED_QUALIFY_OK\n");
    return 0;
}

/* ---- system mode -------------------------------------------------------- */

/* Health contract v3: these services must all be Running, without a death,
 * for HEALTH_WINDOW before the boot is declared healthy:
 *   health   (/bin/healthd)  liveness of the native platform itself
 *   updated  (/bin/updated)  a system that cannot update is not healthy
 *   logd     (/bin/logd)     a system that cannot report is not healthy
 * Changing this list is changing the health contract: bump the version. */
#define CRITICAL_PROFILE  3
#define HEALTH_WINDOW     (3000 * MS)
#define HEALTH_DEADLINE   (90000 * MS)

enum sys_state { SYS_BOOTING, SYS_STARTING, SYS_HEALTHY, SYS_DEGRADED, SYS_RECOVERY, SYS_SHUTDOWN };

static const char *sys_name(enum sys_state s)
{
    switch (s) {
    case SYS_BOOTING:  return "Booting";
    case SYS_STARTING: return "StartingServices";
    case SYS_HEALTHY:  return "Healthy";
    case SYS_DEGRADED: return "Degraded";
    case SYS_RECOVERY: return "RecoveryRequired";
    case SYS_SHUTDOWN: return "ShuttingDown";
    }
    return "?";
}

static enum sys_state sys = SYS_BOOTING;
static bo_u64 logd_boot_pid;

static void sys_to(enum sys_state to, const char *why)
{
    bo_say("[SERVICED] system ");
    bo_say(sys_name(sys));
    bo_say("->");
    bo_say(sys_name(to));
    bo_say(" why=");
    bo_say(why);
    kv("t_ms", (bo_i64)(now() / MS));
    bo_say("\n");
    sys = to;
}

static void say_update_state(bo_handle_t cap)
{
    struct bo_update_state st;
    bo_i64 rc = bo_update_state(cap, &st);
    bo_say("SERVICED_UPDATE_STATE");
    kv("rc", rc);
    if (rc == 0) {
        kv("booted_slot", st.booted_slot);
        kv("boot_reason", st.boot_reason);
        kv("attempts_left", st.attempts_left);
        kv("active_slot", st.active_slot);
        kv("state_a", st.state_a);
        kv("state_b", st.state_b);
        kv("meta_sequence", (bo_i64)st.meta_sequence);
    }
    bo_say("\n");
}

static int system_mode(const struct bo_start *start)
{
    struct bo_handle_info info, info2;
    bo_handle_t cap = 0, install = 0;
    bo_u64 t0, healthy_since = 0;
    int committed = 0, k;

    /* [0] health capability (kept), [1] install capability (passed to
     * updated only). Recognised by kind AND rights, never by position alone. */
    info.rights = 0;
    if (start->bootstrap_count >= 1 && bo_handle_info(start->bootstrap[0], &info) == 0
        && info.kind == BO_KIND_UPDATE && (info.rights & BO_RIGHT_UPDATE_HEALTH))
        cap = start->bootstrap[0];
    if (start->bootstrap_count >= 2 && bo_handle_info(start->bootstrap[1], &info2) == 0
        && info2.kind == BO_KIND_UPDATE && (info2.rights & BO_RIGHT_UPDATE_INSTALL)
        && !(info2.rights & BO_RIGHT_UPDATE_HEALTH))
        install = start->bootstrap[1];
    bo_say("SERVICED_SYSTEM_BEGIN");
    kv("profile", CRITICAL_PROFILE);
    kv("capability", cap ? 1 : 0);
    kv("rights", cap ? (bo_i64)info.rights : 0);
    bo_say("\n");
    if (cap) {
        struct bo_update_state st;
        say_update_state(cap);
        if (bo_update_state(cap, &st) == 0) logd_boot_id = st.boot_id;
    }

    add_service("health", "/bin/healthd", -1);
    svc[0].critical = 1;
    add_service("updated", "/bin/updated", 0);
    svc[1].critical = 1;
    /* updated may install and inspect; it may not pass the capability on
     * (no TRANSFER) nor duplicate it, and it never gets UPDATE_HEALTH. */
    svc[1].extra = install;
    svc[1].extra_mask = BO_RIGHT_INSPECT | BO_RIGHT_UPDATE_INSTALL;
    add_service("logd", "/bin/logd", -1);
    svc[2].critical = 1;
    svc[2].intake = 1;
    /* BOUCHAUD_OTA_ANNEAU3_V1: the OTA agent (verify, download, stage for
     * updated). NOT critical -- the health contract (v3) is unchanged: a
     * system whose agent is absent or dead still boots healthy, it just stops
     * fetching updates. No bootstrap capability beyond its control channel:
     * it never installs (updated does, re-verified by the kernel). */
    add_service("ota", "/bin/bo-ota", -1);
    bo_say("SERVICED_INSTALL_CAPABILITY");
    kv("present", install ? 1 : 0);
    bo_say("\n");
    t0 = now();
    sys_to(SYS_STARTING, cap ? "capability_ok" : "no_update_capability");

    for (;;) {
        int all_running = 1, any_failed = 0;
        bo_u64 t;
        step(100 * MS);
        t = now();
        /* logd learns the boot id as soon as it runs (and again after a
         * restart): every event it stamps then carries the right boot. */
        if (cap && svc[2].state == RUNNING && svc[2].pid != logd_boot_pid) {
            struct bo_update_state st;
            if (bo_update_state(cap, &st) == 0) {
                char b[32]; bo_usize n = put_str(b, 0, sizeof(b), "boot ");
                n = put_u64(b, n, sizeof(b), st.boot_id);
                if (bo_channel_send(svc[2].chan, b, n, 0, 0) >= 0) logd_boot_pid = svc[2].pid;
            }
        }
        for (k = 0; k < svc_count; k++) {
            if (!svc[k].critical) continue;
            if (svc[k].state != RUNNING) all_running = 0;
            if (svc[k].state == FAILED) any_failed = 1;
        }
        if (sys == SYS_RECOVERY) continue;     /* terminal for this boot */
        if (any_failed) { sys_to(SYS_RECOVERY, "critical_service_failed"); continue; }
        if (!all_running) {
            healthy_since = 0;
            if (sys == SYS_HEALTHY) sys_to(SYS_DEGRADED, "critical_service_not_running");
            if (!committed && sys != SYS_DEGRADED && t - t0 > HEALTH_DEADLINE)
                sys_to(SYS_RECOVERY, "health_deadline");
            continue;
        }
        if (!healthy_since) healthy_since = t;
        if (t - healthy_since < HEALTH_WINDOW || sys == SYS_HEALTHY) continue;
        sys_to(SYS_HEALTHY, committed ? "critical_services_back" : "critical_services_running_window");
        if (!committed) {
            bo_i64 rc;
            rc = cap ? bo_update_commit_health(cap) : -BO_E_BAD_HANDLE;
            log_event(rc >= 0 ? 6 : 2, "serviced", 0, "health_commit", rc == 0 ? "result=committed" : rc == 1 ? "result=already_good" : "result=refused");
            committed = 1;
            bo_say("SERVICED_HEALTH_COMMIT");
            kv("rc", rc);
            kv("window_ms", (bo_i64)(HEALTH_WINDOW / MS));
            bo_say(rc == 0 ? " result=committed\n" : rc == 1 ? " result=already_good\n" : " result=refused\n");
            if (cap) say_update_state(cap);
        }
    }
    return 0;   /* not reached: the system supervisor never exits */
}

/* ---- logd qualification (BOUCHAUD_LOGD_V1) -------------------------------- */

#define FLOOD 5000

static int logd_stats(struct service *s, bo_u64 *received, bo_u64 *overwritten, bo_u64 *malformed)
{
    char in[160];
    struct bo_recv_meta meta;
    bo_u64 end = now() + 10000 * MS;
    send_line(s, "stats");
    while (now() < end) {
        bo_i64 got = bo_channel_recv(s->chan, in, sizeof(in), 0, 0, &meta);
        if (got > 0 && prefix(in, (bo_usize)got, "stats ")) {
            bo_usize at = 15;
            *received = parse_u64(in, (bo_usize)got, &at);
            at += 13; *overwritten = parse_u64(in, (bo_usize)got, &at);
            at += 11; *malformed = parse_u64(in, (bo_usize)got, &at);
            return 1;
        }
        if (got < 0) bo_sleep_ns(5 * MS);
    }
    return 0;
}

static int logd_qualify(void)
{
    struct service *logd;
    bo_u64 rec = 0, ov = 0, mal = 0, sent0, drop0, calls0, accepted, refused, burst_refused, t_burst;
    bo_i64 baseline, after, rc;
    int i, b, malformed_sent = 0;
    static const char *bad[3] = { "no fields", "sev=9 svc=x type=y", "sev=1 svc=x" };

    bo_say("[SERVICED] LOGD_QUALIFY_BEGIN\n");
    /* bo_sleep_ns keeps ONE private wait set for the process lifetime (SDK
     * design): create it before the baseline, so the leak check measures
     * logd's handles only (first run: final = baseline + 1, that set). */
    bo_sleep_ns(1000);
    baseline = bo_handle_count();
    sent0 = log_sent; drop0 = log_dropped; calls0 = log_calls;   /* every event from here on counts */
    add_service("logd", "/bin/logd", -1);
    logd = &svc[0];
    logd->intake = 1;
    verdict("logd reaches Running", run_until(logd, RUNNING, 20000 * MS));
    send_line(logd, "boot 4242");
    /* 1. One burst far beyond the intake queue: the producer never blocks,
     *    the queue is bounded, every refusal is counted. */
    t_burst = now();
    for (i = 0; i < FLOOD; i++) log_event(6, "flood", (bo_u64)i, "test", "k=v");
    t_burst = now() - t_burst;
    burst_refused = log_dropped - drop0;
    verdict("backpressure observed (intake queue is bounded)", burst_refused > 0);
    /* 2. Paced bursts that logd can drain: more accepted events than its
     *    ring holds, so the ring MUST overwrite (and count) its oldest. */
    for (b = 0; b < 12; b++) {
        bo_sleep_ns(40 * MS);
        for (i = 0; i < 100; i++) log_event(6, "rythme", (bo_u64)i, "test", "k=v");
    }
    bo_sleep_ns(100 * MS);
    /* 3. Malformed lines are counted by logd, never stored. */
    for (i = 0; i < 3; i++) {
        rc = bo_channel_send(logd_intake, bad[i], bo_strlen(bad[i]), 0, 0);
        if (rc >= 0) malformed_sent++;
    }
    log_event(6, "flood", 0, "after", "");   /* carries dropped=<pending> */
    accepted = log_sent - sent0;
    refused = log_dropped - drop0;
    verdict("logd answers stats after the floods", logd_stats(logd, &rec, &ov, &mal));
    bo_say("LOGD_QUALIFY_COUNTS");
    kv("attempted", (bo_i64)(accepted + refused)); kv("accepted", (bo_i64)accepted);
    kv("refused", (bo_i64)refused); kv("received", (bo_i64)rec);
    kv("overwritten", (bo_i64)ov); kv("malformed", (bo_i64)mal); kv("malformed_sent", malformed_sent);
    kv("burst_us", (bo_i64)(t_burst / 1000));
    bo_say("\n");
    /* Each call returned (no wait in log_event) and is accounted exactly once. */
    verdict("producer never blocked: every send accepted or refused",
            accepted + refused == log_calls - calls0 && log_calls - calls0 >= (bo_u64)FLOOD + 1201);
    verdict("no silent loss: logd received exactly what was accepted", rec == accepted);
    verdict("malformed lines are counted, not stored", malformed_sent == 3 && mal == 3);
    verdict("ring bounded and overwriting: overwritten = received - 256", rec > 256 && ov == rec - 256);
    verdict("logd still Running after the floods", logd->state == RUNNING);
    stop(logd);
    verdict("logd stopped cleanly", run_until(logd, STOPPED, 10000 * MS) && logd->last_exit == 0);
    if (logd_intake) { bo_handle_close(logd_intake); logd_intake = 0; }
    after = bo_handle_count();
    bo_say("LOGD_QUALIFY_HANDLES");
    kv("baseline", baseline); kv("final", after);
    bo_say("\n");
    verdict("every handle released", after == baseline);
    if (failures) { bo_say("LOGD_QUALIFY_FAIL"); kv("failures", failures); bo_say("\n"); return 1; }
    bo_say("LOGD_QUALIFY_OK\n");
    return 0;
}

int bo_main(const struct bo_start *start)
{
    bo_i64 w;
    bo_i64 v = bo_version();
    if (v < 0 || (v >> 16) != BO_ABI_MAJOR || (v & 0xffff) < BO_ABI_MINOR) {
        bo_say("[SERVICED] FAIL abi_version\n");
        return 2;
    }
    w = bo_waitset_create();
    if (w < 0) return 3;
    ws = (bo_handle_t)w;

    if (start->argc > 1 && prefix(start->argv[1], bo_strlen(start->argv[1]), "qualify"))
        return qualify();
    if (start->argc > 1 && prefix(start->argv[1], bo_strlen(start->argv[1]), "system"))
        return system_mode(start);
    if (start->argc > 1 && prefix(start->argv[1], bo_strlen(start->argv[1]), "logd-qualify"))
        return logd_qualify();

    add_service("health", "/bin/healthd", -1);
    for (;;) step(HEARTBEAT_PERIOD);
}
