/* Preuve sous QEMU : UN VRAI processus windowd ring3 lance par l'ABI,
 * recoit des requetes sur un canal natif, repond puis termine proprement.
 * Le navigateur et le jeu ne sont ni charges ni requis pour le test.
 */
#include "bouchaud/sdk.h"
#include "../services/windowd/window_policy.h"

static int check(int ok, const char *reason)
{
    if (ok) return 1;
    bo_say("WINDOWD_PROBE_FAIL ");
    bo_say(reason);
    bo_say("\n");
    return 0;
}

static int roundtrip(bo_handle_t ctrl, bo_handle_t ws,
                     struct wd_request q, struct wd_reply *reply)
{
    struct bo_wait_event e[1];
    struct bo_recv_meta meta;
    bo_i64 len;
    bo_i64 result=bo_channel_send(ctrl,&q,sizeof(q),0,0);
    if (result!=(bo_i64)sizeof(q)) return 0;
    result=bo_waitset_wait(ws,e,1,10000000000UL);
    if (result!=1) return 0;
    len=bo_channel_recv(ctrl,reply,sizeof(*reply),0,0,&meta);
    return len==(bo_i64)sizeof(*reply) && meta.handles==0 &&
           reply->version==WD_VERSION;
}

int bo_main(const struct bo_start *start)
{
    bo_handle_t pair[2]={0,0};
    struct bo_spawn_result proc;
    struct wd_request req;
    struct wd_reply rep;
    struct bo_process_info info;
    struct bo_wait_event events[1];
    /* INSPECT est nécessaire à la validation de droits dans windowd :
     * le canal reste sans DUP ni TRANSFER après atténuation. */
    bo_u32 mask=BO_RIGHT_READ|BO_RIGHT_WRITE|BO_RIGHT_WAIT|BO_RIGHT_INSPECT;
    bo_i64 ws;
    bo_i64 rc;
    wd_u32 first, second;
    int ok=1;
    if (start->bootstrap_count) return 2;
    rc=bo_channel_create(pair);
    if (!check(rc==0,"channel")) return 3;
    rc=bo_process_spawn("/bin/windowd",12,&pair[1],1,&mask,&proc);
    bo_handle_close(pair[1]);
    if (!check(rc==0,"spawn")) return 4;
    ws=bo_waitset_create();
    if (!check(ws>0 && bo_waitset_add((bo_handle_t)ws,pair[0],1)==0,"waitset")) return 5;
    req=(struct wd_request){WD_VERSION,WD_OPEN,0,-20,-30,640,400};
    ok &= check(roundtrip(pair[0],(bo_handle_t)ws,req,&rep),"open ipc");
    ok &= check(rep.verdict==WD_OK && rep.x==0 && rep.y==0 &&
                rep.width==640 && rep.height==400 && rep.focus_id==rep.id,
                "window identity and clamp");
    first=rep.id;
    req=(struct wd_request){WD_VERSION,WD_OPEN,0,100,50,500,300};
    ok &= check(roundtrip(pair[0],(bo_handle_t)ws,req,&rep),"second open");
    second=rep.id;
    ok &= check(rep.verdict==WD_OK && second!=first && rep.focus_id==second,
                "separate window ids");
    req=(struct wd_request){WD_VERSION,WD_FOCUS,first,0,0,0,0};
    ok &= check(roundtrip(pair[0],(bo_handle_t)ws,req,&rep),"focus ipc");
    ok &= check(rep.verdict==WD_OK && rep.focus_id==first,"focus state");
    req=(struct wd_request){WD_VERSION,WD_MOVE,first,5000,5000,0,0};
    ok &= check(roundtrip(pair[0],(bo_handle_t)ws,req,&rep),"move ipc");
    ok &= check(rep.verdict==WD_OK && rep.x==640 && rep.y==320,"clamped move");
    req=(struct wd_request){WD_VERSION,WD_CLOSE,first,0,0,0,0};
    ok &= check(roundtrip(pair[0],(bo_handle_t)ws,req,&rep),"close ipc");
    ok &= check(rep.verdict==WD_OK && rep.focus_id==second,"focus recovery");
    req=(struct wd_request){WD_VERSION,WD_QUERY,first,0,0,0,0};
    ok &= check(roundtrip(pair[0],(bo_handle_t)ws,req,&rep),"query closed ipc");
    ok &= check(rep.verdict==WD_NOT_FOUND,"closed window invalid");
    /* Verifie aussi que la fermeture du processus laisse le parent vivant. */
    req=(struct wd_request){WD_VERSION,WD_SHUTDOWN,0,0,0,0,0};
    if (bo_channel_send(pair[0],&req,sizeof(req),0,0)!=(bo_i64)sizeof(req)) ok=0;
    rc=bo_waitset_wait((bo_handle_t)ws,events,1,10000000000UL);
    if (rc==1) {
        char bye[8];
        struct bo_recv_meta meta;
        rc=bo_channel_recv(pair[0],bye,sizeof(bye),0,0,&meta);
        ok &= check(rc==3 && bye[0]=='B' && bye[1]=='Y' && bye[2]=='E',"shutdown ack");
    } else ok=0;
    /* Observation de terminaison, pas de collecte d'un zombie par handle. */
    bo_i64 pws=bo_waitset_create();
    if (pws>0 && bo_waitset_add((bo_handle_t)pws,proc.process,2)==0) {
        rc=bo_waitset_wait((bo_handle_t)pws,events,1,10000000000UL);
        ok &= check(rc==1 && (events[0].signals & BO_SIGNAL_TERMINATED),
                    "child termination");
    } else ok=0;
    if (bo_process_info(proc.process,&info)==0)
        ok &= check(info.state==BO_PROCESS_TERMINATED && info.exit_code==0,
                    "child clean exit");
    else ok=0;
    bo_handle_close(proc.process);
    bo_handle_close(pair[0]);
    if (pws>0) bo_handle_close((bo_handle_t)pws);
    if (ws>0) bo_handle_close((bo_handle_t)ws);
    if (!ok) { bo_say("WINDOWD_PROBE_FAIL\n"); return 10; }
    bo_say("WINDOWD_RING3_IPC_OK\n");
    return 0;
}
