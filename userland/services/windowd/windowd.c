/* windowd: native ring3 window policy broker.
 * Bootstrap channel zero belongs to the trusted compositor/supervisor.
 * Additional bootstrap channels represent distinct untrusted applications.
 * A channel's principal is assigned by the spawning supervisor, never by
 * an untrusted message. Disconnect revokes all windows owned by that channel.
 */
#include "bouchaud/sdk.h"
#include "window_policy.h"

static int revoke(struct wd_state *state, wd_u32 principal)
{
    struct wd_request req={WD_VERSION,WD_CLOSE_OWNED,0,0,0,0,0};
    return wd_apply(state,principal,&req).verdict==WD_OK;
}

int bo_main(const struct bo_start *start)
{
    bo_handle_t controls[BO_START_MAX_BOOTSTRAP];
    wd_u32 count, i, active;
    bo_i64 waitset;
    struct wd_state state;
    struct bo_wait_event ev[BO_START_MAX_BOOTSTRAP];
    wd_init(&state,1280,720);
    if (start->bootstrap_count<1 ||
        start->bootstrap_count>BO_START_MAX_BOOTSTRAP) {
        bo_say("WINDOWD_RING3_FAIL bootstrap\n");
        return 2;
    }
    count=(wd_u32)start->bootstrap_count;
    active=count;
    for (i=0; i<count; ++i) {
        struct bo_handle_info info;
        controls[i]=start->bootstrap[i];
        if (bo_handle_info(controls[i],&info)!=0 ||
            info.kind!=BO_KIND_CHANNEL ||
            (info.rights & (BO_RIGHT_READ|BO_RIGHT_WRITE|BO_RIGHT_WAIT)) !=
            (BO_RIGHT_READ|BO_RIGHT_WRITE|BO_RIGHT_WAIT) ||
            (info.rights & (BO_RIGHT_DUP|BO_RIGHT_TRANSFER)) !=0) {
            bo_say("WINDOWD_RING3_FAIL droits\n");
            return 3;
        }
    }
    waitset=bo_waitset_create();
    if (waitset<0) {
        bo_say("WINDOWD_RING3_FAIL attente\n");
        return 4;
    }
    for (i=0; i<count; ++i)
        if (bo_waitset_add((bo_handle_t)waitset,controls[i],i+1)!=0) {
            bo_say("WINDOWD_RING3_FAIL attente\n");
            return 4;
        }
    bo_say("WINDOWD_RING3_READY\n");
    while (active) {
        bo_i64 n=bo_waitset_wait((bo_handle_t)waitset,ev,
                                BO_START_MAX_BOOTSTRAP,5000000000UL);
        wd_u32 e;
        if (n==0 || n==-BO_E_TIMED_OUT) continue;
        if (n<0) return 5;
        for (e=0; e<(wd_u32)n; ++e) {
            wd_u32 slot;
            wd_u32 principal;
            bo_i64 received;
            struct wd_request req;
            struct wd_reply reply;
            struct bo_recv_meta meta;
            if (ev[e].key<1 || ev[e].key>count) return 6;
            slot=(wd_u32)ev[e].key-1;
            if (!controls[slot]) continue;
            principal=slot+1; /* 1 is the trusted supervisor. */
            if (!(ev[e].signals & BO_SIGNAL_READABLE)) {
                if (ev[e].signals & BO_SIGNAL_PEER_CLOSED) {
                    if (!revoke(&state,principal)) return 7;
                    bo_waitset_remove((bo_handle_t)waitset,slot+1);
                    bo_handle_close(controls[slot]);
                    controls[slot]=0;
                    --active;
                } else if (bo_sleep_ns(1000000UL)<0) return 10;
                continue;
            }
            received=bo_channel_recv(controls[slot],&req,sizeof(req),
                                     0,0,&meta);
            if (received==-BO_E_PEER_CLOSED) {
                if (!revoke(&state,principal)) return 7;
                bo_waitset_remove((bo_handle_t)waitset,slot+1);
                bo_handle_close(controls[slot]);
                controls[slot]=0;
                --active;
                continue;
            }
            if (received==-BO_E_WOULD_BLOCK) continue;
            if (received!=(bo_i64)sizeof(req) || meta.handles!=0) {
                /* A malformed client must not terminate the shared broker or
                 * invalidate windows belonging to other applications. */
                bo_say("WINDOWD_RING3_CLIENT_PROTOCOL_REVOKED\n");
                if (!revoke(&state,principal)) return 7;
                bo_waitset_remove((bo_handle_t)waitset,slot+1);
                bo_handle_close(controls[slot]);
                controls[slot]=0;
                --active;
                continue;
            }
            if (req.version==WD_VERSION && req.operation==WD_SHUTDOWN) {
                /* Only the supervisor may terminate the broker. */
                if (principal==1) {
                    if (bo_channel_send(controls[slot],"BYE",3,0,0)!=3)
                        return 8;
                    return 0;
                }
                reply=(struct wd_reply){WD_VERSION,WD_DENIED,0,
                                        state.focus_id,0,0,0,0};
            } else {
                reply=wd_apply(&state,principal,&req);
            }
            if (bo_channel_send(controls[slot],&reply,sizeof(reply),0,0)
                !=(bo_i64)sizeof(reply)) {
                /* Peer disconnect or backpressure failure is scoped to the
                 * channel; other application sessions remain operational. */
                if (!revoke(&state,principal)) return 9;
                bo_waitset_remove((bo_handle_t)waitset,slot+1);
                bo_handle_close(controls[slot]);
                controls[slot]=0;
                --active;
            }
        }
    }
    return 0;
}
