/*
 * native-cycles-probe -- hostile reference cycles through native objects.
 *
 * Two ways ring 3 could tie the kernel in a knot:
 *
 *   1. a wait set watching itself (or W1 -> W2 -> W1). WaitSet::poll holds
 *      the wait set's spin lock while asking each watched object for its
 *      signals; asking ITSELF re-takes the same non-reentrant lock and the
 *      CPU spins forever in the kernel;
 *   2. a channel endpoint sent through its own channel: the message queue
 *      then owns the endpoint that owns the queue, and neither is ever freed.
 *
 * Both must be REFUSED at the call that would create the cycle. Every line
 * below is a verdict; the final marker only prints if the kernel came back
 * from every call.
 */
#include "bouchaud/sdk.h"

static int failures;

static void expect(const char *what, bo_i64 got, bo_i64 want)
{
    bo_puts("[NATIVE-CYCLES] ");
    bo_puts(what);
    bo_puts(got == want ? " ok got=" : " FAIL got=");
    bo_put_i64(got);
    bo_puts(" want=");
    bo_put_i64(want);
    bo_puts("\n");
    if (got != want) failures++;
}

int bo_main(const struct bo_start *start)
{
    struct bo_wait_event ev[4];
    bo_handle_t pair[2];
    bo_handle_t moved[1];
    bo_i64 ws1, ws2, r;
    (void)start;

    bo_puts("[NATIVE-CYCLES] BEGIN\n");
    ws1 = bo_waitset_create();
    ws2 = bo_waitset_create();
    if (ws1 < 0 || ws2 < 0) { bo_puts("[NATIVE-CYCLES] FAIL create\n"); return 1; }

    r = bo_waitset_add((bo_handle_t)ws1, (bo_handle_t)ws1, 1);
    expect("waitset_self_add", r, -BO_E_WRONG_TYPE);
    if (r == 0) {
        /* Unfixed kernel: this poll never returns. */
        bo_puts("[NATIVE-CYCLES] CYCLE_ACCEPTED polling-self\n");
        r = bo_waitset_poll((bo_handle_t)ws1, ev, 4);
        bo_puts("[NATIVE-CYCLES] POLL_RETURNED\n");
    }

    r = bo_waitset_add((bo_handle_t)ws2, (bo_handle_t)ws1, 2);
    expect("waitset_nested_add", r, -BO_E_WRONG_TYPE);

    if (bo_channel_create(pair) != 0) { bo_puts("[NATIVE-CYCLES] FAIL channel\n"); return 1; }
    moved[0] = pair[0];
    r = bo_channel_send(pair[0], "x", 1, moved, 1);
    expect("channel_send_self_endpoint", r, -BO_E_INVALID_ARGUMENT);
    moved[0] = pair[1];
    r = bo_channel_send(pair[0], "x", 1, moved, 1);
    expect("channel_send_peer_endpoint", r, -BO_E_INVALID_ARGUMENT);

    /* The refused sends must not have consumed or corrupted anything. */
    r = bo_channel_send(pair[0], "y", 1, 0, 0);
    expect("channel_still_usable", r, 1);

    bo_handle_close(pair[0]);
    bo_handle_close(pair[1]);
    bo_handle_close((bo_handle_t)ws1);
    bo_handle_close((bo_handle_t)ws2);

    if (failures) {
        bo_puts("[NATIVE-CYCLES] FAIL\n");
        return 1;
    }
    bo_puts("[NATIVE-CYCLES] OK\n");
    return 0;
}
