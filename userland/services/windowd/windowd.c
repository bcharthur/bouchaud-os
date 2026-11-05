/* windowd — premiere extraction de politique de fenetres en ring 3.
 *
 * Pas de framebuffer ni de code Ladybird/DOOM. Un unique canal de controle
 * attenue constitue l'autorite du client (principal=1). En cas de fermeture
 * du canal, la politique est detruite avec le processus.
 * Ce programme n'est pas encore le WM de production Atelier.
 */
#include "bouchaud/sdk.h"
#include "window_policy.h"

int bo_main(const struct bo_start *start)
{
    bo_handle_t control;
    bo_i64 waitset;
    struct wd_state state;
    struct bo_wait_event ev[1];
    struct bo_recv_meta meta;
    struct bo_handle_info info;
    wd_init(&state,1280,720);
    if (start->bootstrap_count != 1) {
        bo_say("WINDOWD_RING3_FAIL bootstrap\n");
        return 2;
    }
    control=start->bootstrap[0];
    if (bo_handle_info(control,&info)!=0 ||
        info.kind!=BO_KIND_CHANNEL ||
        (info.rights & (BO_RIGHT_READ|BO_RIGHT_WRITE|BO_RIGHT_WAIT)) !=
        (BO_RIGHT_READ|BO_RIGHT_WRITE|BO_RIGHT_WAIT) ||
        (info.rights & (BO_RIGHT_DUP|BO_RIGHT_TRANSFER)) !=0) {
        bo_say("WINDOWD_RING3_FAIL droits\n");
        return 3;
    }
    waitset=bo_waitset_create();
    if (waitset < 0 || bo_waitset_add((bo_handle_t)waitset,control,1) != 0) {
        bo_say("WINDOWD_RING3_FAIL attente\n");
        return 4;
    }
    bo_say("WINDOWD_RING3_READY\n");
    for (;;) {
        struct wd_request req;
        struct wd_reply reply;
        bo_i64 count=bo_waitset_wait((bo_handle_t)waitset,ev,1,5000000000UL);
        if (count==0 || count==-BO_E_TIMED_OUT) continue;
        if (count<0) return 5;
        count=bo_channel_recv(control,&req,sizeof(req),0,0,&meta);
        if (count==-BO_E_PEER_CLOSED) return 0;
        if (count==-BO_E_WOULD_BLOCK) continue;
        if (count<0) return 6;
        if (count!=(bo_i64)sizeof(req) || meta.handles!=0) {
            bo_say("WINDOWD_RING3_FAIL protocole\n");
            return 7; /* refuse formes arbitraires ; pas de reinterpretation */
        }
        if (req.version==WD_VERSION && req.operation==WD_SHUTDOWN) {
            if (bo_channel_send(control,"BYE",3,0,0)!=3) return 8;
            return 0;
        }
        reply=wd_apply(&state,1,&req);
        if (bo_channel_send(control,&reply,sizeof(reply),0,0)!=(bo_i64)sizeof(reply))
            return 9;
    }
}
