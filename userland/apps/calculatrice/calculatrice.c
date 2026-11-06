/* Premiere application native Bouchaud en ring3 — calculatrice sans GUI.
 * Mode sans bootstrap : autotest observable via marqueur serie en QEMU.
 * Mode IPC : un unique canal bootstrap READ|WRITE|WAIT, requetes
 * "ADD 2 3", "SUB 2 3", "MUL 2 3", "DIV 6 2" ; reponses "OK 5" ou ERR.
 */
#include "bouchaud/sdk.h"
#include "calculatrice_core.h"

static void renvoie(bo_handle_t canal, const char *line, bo_usize n) {
    calc_i64 resultat;
    enum calc_status rc = calc_execute(line, n, &resultat);
    char reply[72];
    bo_usize sz = 0;
    const char *erreur;
    if (rc == CALC_OK) {
        reply[sz++]='O'; reply[sz++]='K'; reply[sz++]=' ';
        sz += calc_format(resultat, reply + sz, sizeof(reply) - sz);
    } else {
        erreur = rc == CALC_DIV_ZERO ? "ERR DIV_ZERO" :
                 rc == CALC_OVERFLOW ? "ERR OVERFLOW" : "ERR FORMAT";
        for (bo_usize j = 0; erreur[j] && sz < sizeof(reply); j++)
            reply[sz++] = erreur[j];
    }
    (void)bo_channel_send(canal, reply, sz, 0, 0);
}

int bo_main(const struct bo_start *start) {
    calc_i64 v=0;
    if (!start->bootstrap_count) {
        /* VRAI processus ring3 avec le SDK ; ne prouve PAS la GUI native. */
        if (calc_execute("ADD 2 3", 7, &v) != CALC_OK || v != 5 ||
            calc_execute("DIV 1 0", 7, &v) != CALC_DIV_ZERO ||
            calc_execute("MUL 7 -2", 8, &v) != CALC_OK || v != -14) {
            bo_say("[CALCULATRICE] FAIL selftest\n");
            return 2;
        }
        bo_say("CALCULATRICE_RING3_OK\n");
        return 0;
    }
    if (start->bootstrap_count != 1) {
        bo_say("[CALCULATRICE] FAIL bootstrap_count\n");
        return 3;
    }
    {
        bo_handle_t canal = start->bootstrap[0];
        bo_i64 ws = bo_waitset_create();
        struct bo_wait_event events[1];
        char command[128];
        struct bo_recv_meta meta;
        if (ws < 0 || bo_waitset_add((bo_handle_t)ws, canal, 1) != 0) return 4;
        for (;;) {
            bo_i64 n = bo_waitset_wait((bo_handle_t)ws, events, 1, 5000000000UL);
            if (n == -BO_E_TIMED_OUT || n == 0) continue;
            if (n < 0) return 5;
            n = bo_channel_recv(canal, command, sizeof(command), 0, 0, &meta);
            if (n == -BO_E_PEER_CLOSED) return 0;
            if (n == -BO_E_WOULD_BLOCK) continue;
            if (n < 0) return 6;
            if (n == 4 && command[0]=='s' && command[1]=='t' &&
                command[2]=='o' && command[3]=='p') return 0;
            renvoie(canal, command, (bo_usize)n);
        }
    }
}
