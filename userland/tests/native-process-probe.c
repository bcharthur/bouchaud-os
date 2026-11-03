/*
 * native-process-probe -- who owns a dead child? (BOUCHAUD_NATIVE_PROCESS_OWNERSHIP_V1)
 *
 * Contract under test:
 *   - the ZOMBIE of a child launched by bo_process_spawn belongs to its POSIX
 *     parent, exactly like a fork child: wait4 collects it and returns its
 *     status, whatever happened to native Process handles;
 *   - a native Process handle is an OBSERVER: TERMINATED + exit code, never
 *     the zombie. Closing it (or dying with it) never collects anything;
 *   - Bouchaud has NO reparenting: a child whose parent died is not adopted;
 *     a non-parent holding its handle observes it but cannot wait4 it.
 *
 * Modes: started with no bootstrap handle -> parent (the scenario);
 * started with one bootstrap channel -> "intermediate" used by case 3.
 */
#include "bouchaud/sdk.h"

#define NR_WAIT4 61
#define ECHILD 10
#define WNOHANG 1

static int failures;

static bo_i64 wait4(bo_i64 pid, int *status, int options)
{
    return bo__sys6(NR_WAIT4, (bo_u64)pid, (bo_u64)status, (bo_u64)options, 0, 0, 0);
}

static void check(const char *what, int ok, bo_i64 detail)
{
    bo_say(ok ? "[NATIVE-PROCESS] ok   " : "[NATIVE-PROCESS] FAIL ");
    bo_say(what);
    bo_say_kv("detail", detail);
    bo_say("\n");
    if (!ok) failures++;
}

/* Wait until `h` shows one of `want` (bounded, 10 s). bo_waitset_wait
 * returns 0 when a POSIX signal (here: SIGCHLD of earlier children)
 * interrupts the wait; the ABI says the caller loops, so we loop. */
static int wait_signal(bo_handle_t h, bo_u32 want)
{
    struct bo_wait_event ev[1];
    bo_i64 ws = bo_waitset_create();
    bo_i64 end = bo_time_now_ns() + 10000000000L;
    int ok = 0;
    if (ws < 0) return 0;
    if (bo_waitset_add((bo_handle_t)ws, h, 1) == 0) {
        while (!ok) {
            bo_i64 left = end - bo_time_now_ns();
            bo_i64 n;
            if (left <= 0) break;
            n = bo_waitset_wait((bo_handle_t)ws, ev, 1, (bo_u64)left);
            if (n < 0) break;
            ok = n == 1 && (ev[0].signals & want);
        }
    }
    bo_handle_close((bo_handle_t)ws);
    return ok;
}

static int wait_terminated(bo_handle_t proc) { return wait_signal(proc, BO_SIGNAL_TERMINATED); }

static bo_i64 spawn_healthd(bo_handle_t *chan_out, struct bo_spawn_result *r, int with_channel)
{
    bo_handle_t pair[2];
    bo_u32 mask = BO_RIGHT_READ | BO_RIGHT_WRITE | BO_RIGHT_WAIT;
    bo_i64 rc;
    if (!with_channel)
        return bo_process_spawn("/bin/healthd", 12, 0, 0, 0, r);
    if (bo_channel_create(pair) != 0) return -1;
    rc = bo_process_spawn("/bin/healthd", 12, &pair[1], 1, &mask, r);
    bo_handle_close(pair[1]);
    if (rc != 0) { bo_handle_close(pair[0]); return rc; }
    *chan_out = pair[0];
    return 0;
}

/* exit code for "status" in the wait(2) encoding, -1 if signaled */
static int exit_status(int st) { return (st & 0x7f) == 0 ? (st >> 8) & 0xff : -1; }
static int term_signal(int st) { return st & 0x7f; }

static int intermediate(const struct bo_start *start)
{
    /* Case 3, middle generation: spawn a long-lived child, hand its process
     * handle to our parent, then die while the child is still running. */
    bo_handle_t up = start->bootstrap[0];
    bo_handle_t chan = 0;
    struct bo_spawn_result r;
    bo_handle_t give[1];
    if (spawn_healthd(&chan, &r, 1) != 0) return 3;
    give[0] = r.process;
    if (bo_channel_send(up, "child", 5, give, 1) != 5) return 4;
    /* Dying releases our table: our end of `chan` closes, healthd sees
     * PEER_CLOSED and exits 0 -- after we are gone. */
    return 0;
}

static void case_exit_then_wait4(void)
{
    bo_handle_t chan = 0;
    struct bo_spawn_result r;
    struct bo_process_info info;
    int st = -1;
    bo_i64 got;
    if (spawn_healthd(&chan, &r, 1) != 0) { check("c1 spawn", 0, -1); return; }
    bo_channel_send(chan, "stop", 4, 0, 0);
    check("c1 TERMINATED observed", wait_terminated(r.process), (bo_i64)r.pid);
    bo_process_info(r.process, &info);
    check("c1 info exit_code 0", info.state == BO_PROCESS_TERMINATED && info.exit_code == 0, info.exit_code);
    got = wait4((bo_i64)r.pid, &st, 0);
    check("c1 wait4 collects the child", got == (bo_i64)r.pid, got);
    check("c1 wait4 status = exit 0", exit_status(st) == 0, st);
    bo_handle_close(r.process);
    bo_handle_close(chan);
}

static void case_exit_close_then_wait4(void)
{
    bo_handle_t chan = 0;
    struct bo_spawn_result r;
    struct bo_process_info info;
    int st = -1;
    bo_i64 got;
    if (spawn_healthd(&chan, &r, 1) != 0) { check("c2 spawn", 0, -1); return; }
    bo_channel_send(chan, "crash", 5, 0, 0);
    check("c2 TERMINATED observed", wait_terminated(r.process), (bo_i64)r.pid);
    bo_process_info(r.process, &info);
    check("c2 info exit_code = fault", info.exit_code != 0, info.exit_code);
    /* Last handle closed BEFORE wait4: must not steal the zombie. */
    bo_handle_close(r.process);
    bo_handle_close(chan);
    got = wait4((bo_i64)r.pid, &st, 0);
    check("c2 wait4 after closing the handle still collects", got == (bo_i64)r.pid, got);
    check("c2 wait4 status = SIGSEGV", term_signal(st) == 11, st);
}

static void case_parent_dies_first(void)
{
    bo_handle_t pair[2];
    struct bo_spawn_result mid;
    bo_handle_t got_handle[1] = { 0 };
    char buf[16];
    struct bo_recv_meta meta;
    struct bo_process_info info;
    int st = -1;
    bo_i64 ws, got;

    if (bo_channel_create(pair) != 0) { check("c3 channel", 0, -1); return; }
    if (bo_process_spawn("/bin/native-process-probe", 25, &pair[1], 1, 0, &mid) != 0) {
        check("c3 spawn intermediate", 0, -1);
        return;
    }
    bo_handle_close(pair[1]);
    ws = bo_waitset_create();
    check("c3 intermediate sent the grandchild handle", wait_signal(pair[0], BO_SIGNAL_READABLE), 0);
    got = bo_channel_recv(pair[0], buf, sizeof(buf), got_handle, 1, &meta);
    check("c3 received grandchild handle", got == 5 && meta.handles == 1, got);
    if (got != 5 || meta.handles != 1) {
        bo_handle_close((bo_handle_t)ws);
        bo_handle_close(pair[0]);
        bo_handle_close(mid.process);
        return;
    }
    check("c3 intermediate (its parent) terminated", wait_terminated(mid.process), (bo_i64)mid.pid);
    got = wait4((bo_i64)mid.pid, &st, 0);
    check("c3 wait4 collects the intermediate", got == (bo_i64)mid.pid, got);
    /* The grandchild outlives its parent and exits when its channel closes. */
    check("c3 grandchild TERMINATED seen by a non-parent", wait_terminated(got_handle[0]), 0);
    bo_process_info(got_handle[0], &info);
    check("c3 grandchild exit_code 0", info.exit_code == 0, info.exit_code);
    got = wait4((bo_i64)info.pid, &st, WNOHANG);
    check("c3 non-parent cannot wait4 (no reparenting)", got == -ECHILD, got);
    bo_handle_close(got_handle[0]);
    bo_handle_close((bo_handle_t)ws);
    bo_handle_close(pair[0]);
    bo_handle_close(mid.process);
}

static void case_dies_before_handle(void)
{
    int i, ok_term = 0, ok_code = 0, ok_wait = 0;
    for (i = 0; i < 20; i++) {
        struct bo_spawn_result r;
        struct bo_process_info info;
        int st = -1;
        /* healthd without its bootstrap channel exits(2) on its first
         * instructions: often before bo_process_spawn has returned. */
        if (spawn_healthd(0, &r, 0) != 0) continue;
        ok_term += wait_terminated(r.process);
        bo_process_info(r.process, &info);
        ok_code += info.exit_code == 2;
        ok_wait += wait4((bo_i64)r.pid, &st, 0) == (bo_i64)r.pid && exit_status(st) == 2;
        bo_handle_close(r.process);
    }
    check("c4 early death never lost (20 spawns, TERMINATED)", ok_term == 20, ok_term);
    check("c4 early death exit_code 2 (20 spawns)", ok_code == 20, ok_code);
    check("c4 early death collected by wait4 (20 spawns)", ok_wait == 20, ok_wait);
}

static void case_duplicated_handles(void)
{
    bo_handle_t chan = 0;
    struct bo_spawn_result r;
    struct bo_process_info info;
    bo_i64 dup, rc;
    int st = -1;
    if (spawn_healthd(&chan, &r, 1) != 0) { check("c5 spawn", 0, -1); return; }
    dup = bo_handle_dup(r.process, BO_RIGHT_WAIT | BO_RIGHT_INSPECT);
    check("c5 dup (WAIT|INSPECT)", dup > 0, dup);
    rc = bo_process_kill((bo_handle_t)dup);
    check("c5 kill through a dup without SIGNAL is denied", rc == -BO_E_ACCESS_DENIED, rc);
    bo_handle_close(r.process);                 /* original gone, dup remains */
    bo_channel_send(chan, "stop", 4, 0, 0);
    check("c5 TERMINATED observed through the dup", wait_terminated((bo_handle_t)dup), 0);
    bo_process_info((bo_handle_t)dup, &info);
    check("c5 dup reports the same pid and exit 0", info.pid == r.pid && info.exit_code == 0, info.exit_code);
    bo_handle_close((bo_handle_t)dup);          /* last handle, no wait4 yet */
    rc = wait4((bo_i64)r.pid, &st, 0);
    check("c5/c6 last handle closed without wait4: zombie still parent's", rc == (bo_i64)r.pid, rc);
    bo_handle_close(chan);
}

int bo_main(const struct bo_start *start)
{
    bo_i64 base, end, left;
    if (start->bootstrap_count == 1) return intermediate(start);

    bo_say("[NATIVE-PROCESS] BEGIN\n");
    base = bo_handle_count();
    case_exit_then_wait4();
    case_exit_close_then_wait4();
    case_parent_dies_first();
    case_dies_before_handle();
    case_duplicated_handles();
    left = wait4(-1, 0, WNOHANG);
    check("no child left uncollected (wait4(-1) = ECHILD)", left == -ECHILD, left);
    end = bo_handle_count();
    check("handles back to baseline", end == base, end);
    if (failures) {
        bo_say("[NATIVE-PROCESS] FAIL");
        bo_say_kv("failures", failures);
        bo_say("\n");
        return 1;
    }
    bo_say("[NATIVE-PROCESS] OK\n");
    return 0;
}
