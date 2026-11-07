/* windowd — politique de fenetres PURE, partagee avec les tests hote.
 *
 * Ni framebuffer, ni Ladybird, ni DOOM, ni Linux, ni appel noyau.
 * Ce contrat ne donne aucun acces materiel aux applications.
 * Un canal = un principal authentifie par son handle de bootstrap.
 * Le principal est attribue par le superviseur, jamais choisi par la requete.
 */
#ifndef BOUCHAUD_WINDOWD_POLICY_H
#define BOUCHAUD_WINDOWD_POLICY_H

typedef unsigned int wd_u32;
typedef int wd_i32;

enum { WD_VERSION=1, WD_MAX_WINDOWS=16 };
enum wd_operation {
    WD_OPEN=1, WD_MOVE=2, WD_RESIZE=3, WD_FOCUS=4, WD_CLOSE=5,
    WD_QUERY=6, WD_SHUTDOWN=7, WD_FOCUS_NEXT=8, WD_CLOSE_OWNED=9, WD_MINIMIZE=10, WD_RESTORE=11, WD_MAXIMIZE=12, WD_UNMAXIMIZE=13
};
enum wd_verdict {
    WD_OK=0, WD_INVALID=1, WD_NOT_FOUND=2, WD_DENIED=3, WD_NO_SLOTS=4
};

struct wd_request {
    wd_u32 version, operation, id;
    wd_i32 x, y;
    wd_u32 width, height;
};
struct wd_reply {
    wd_u32 version, verdict, id, focus_id;
    wd_i32 x, y;
    wd_u32 width, height;
};
struct wd_window {
    wd_u32 id, owner, alive, minimized, maximized;
    wd_i32 saved_x, saved_y;
    wd_u32 saved_width, saved_height;
    wd_i32 x, y;
    wd_u32 width, height;
};
struct wd_state {
    wd_u32 next_id, focus_id, screen_width, screen_height;
    wd_u32 focus_history[WD_MAX_WINDOWS], focus_count;
    struct wd_window windows[WD_MAX_WINDOWS];
};

typedef char wd_request_is_28_bytes[(sizeof(struct wd_request)==28)?1:-1];
typedef char wd_reply_is_32_bytes[(sizeof(struct wd_reply)==32)?1:-1];

static inline void wd_init(struct wd_state *state, wd_u32 width, wd_u32 height)
{
    wd_u32 i;
    state->next_id=1; state->focus_id=0;
    state->screen_width=width; state->screen_height=height;
    state->focus_count=0;
    for (i=0; i<WD_MAX_WINDOWS; ++i) state->focus_history[i]=0;
    for (i=0; i<WD_MAX_WINDOWS; ++i) {
        struct wd_window *w=&state->windows[i];
        w->alive=0; w->minimized=0; w->maximized=0;
        w->saved_x=0; w->saved_y=0; w->saved_width=0; w->saved_height=0; w->id=0; w->owner=0; w->x=0; w->y=0; w->width=0; w->height=0;
    }
}

static inline struct wd_window *wd_find(struct wd_state *s, wd_u32 id)
{
    wd_u32 i;
    for (i=0; i<WD_MAX_WINDOWS; ++i)
        if (s->windows[i].alive && s->windows[i].id==id) return &s->windows[i];
    return 0;
}

/* Historique MRU borne : restaurer le dernier focus reel. */
static inline void wd_forget_focus(struct wd_state *s, wd_u32 id)
{
    wd_u32 i, j=0;
    for (i=0; i<s->focus_count; ++i)
        if (s->focus_history[i]!=id) s->focus_history[j++]=s->focus_history[i];
    s->focus_count=j;
}
static inline void wd_set_focus(struct wd_state *s, wd_u32 id)
{
    wd_forget_focus(s,id);
    if (s->focus_count==WD_MAX_WINDOWS) {
        wd_u32 i;
        for (i=1; i<WD_MAX_WINDOWS; ++i)
            s->focus_history[i-1]=s->focus_history[i];
        --s->focus_count;
    }
    s->focus_history[s->focus_count++]=id;
    s->focus_id=id;
}

static inline wd_i32 wd_clamp(wd_i32 coord, wd_u32 dim, wd_u32 screen)
{
    if (coord < 0) return 0;
    if ((wd_u32)coord > screen-dim) return (wd_i32)(screen-dim);
    return coord;
}

static inline void wd_report(struct wd_reply *r, struct wd_state *s,
                             const struct wd_window *w)
{
    r->focus_id=s->focus_id;
    if (!w) { r->x=0; r->y=0; r->width=0; r->height=0; return; }
    r->x=w->x; r->y=w->y; r->width=w->width; r->height=w->height;
}

/* Aucun opcode ne peut attribuer une fenetre d'un autre principal. */
static inline struct wd_reply wd_apply(struct wd_state *s, wd_u32 principal,
                                       const struct wd_request *q)
{
    struct wd_reply r={WD_VERSION, WD_INVALID, 0, s->focus_id, 0,0,0,0};
    struct wd_window *w=0;
    wd_u32 i;
    if (principal==0 || q->version!=WD_VERSION) return r;
    if (q->operation==WD_OPEN) {
        if (q->id || q->width==0 || q->height==0 ||
            q->width > s->screen_width || q->height > s->screen_height)
            return r;
        for (i=0; i<WD_MAX_WINDOWS; ++i)
            if (!s->windows[i].alive) { w=&s->windows[i]; break; }
        if (!w || s->next_id==0) {
            r.verdict=WD_NO_SLOTS;
            return r;
        }
        w->alive=1; w->minimized=0; w->maximized=0; w->owner=principal; w->id=s->next_id++;
        w->width=q->width; w->height=q->height;
        w->x=wd_clamp(q->x, w->width, s->screen_width);
        w->y=wd_clamp(q->y, w->height, s->screen_height);
        wd_set_focus(s,w->id);
    } else if (q->operation==WD_CLOSE_OWNED) {
        /* Application teardown: remove only windows belonging to this
         * authenticated principal; never close another application's UI. */
        if (q->id) return r;
        for (i=0; i<WD_MAX_WINDOWS; ++i) {
            if (s->windows[i].alive && s->windows[i].owner==principal) {
                wd_forget_focus(s,s->windows[i].id);
                s->windows[i].alive=0;
            }
        }
        if (!wd_find(s,s->focus_id))
            s->focus_id=s->focus_count ? s->focus_history[s->focus_count-1] : 0;
        r.verdict=WD_OK;
        wd_report(&r,s,0);
        return r;
    } else if (q->operation==WD_FOCUS_NEXT) {
        /* Parcours circulaire par emplacement, limite aux fenetres du canal.
         * Ne jamais donner le focus a une fenetre d'un autre principal. */
        wd_u32 start=WD_MAX_WINDOWS;
        for (i=0; i<WD_MAX_WINDOWS; ++i)
            if (s->windows[i].alive && s->windows[i].id==s->focus_id) {
                start=i; break;
            }
        for (i=1; i<=WD_MAX_WINDOWS; ++i) {
            wd_u32 slot=(start==WD_MAX_WINDOWS ? i-1 : start+i)%WD_MAX_WINDOWS;
            if (s->windows[slot].alive && !s->windows[slot].minimized &&
                s->windows[slot].owner==principal) {
                w=&s->windows[slot]; break;
            }
        }
        if (!w) { r.verdict=WD_NOT_FOUND; return r; }
        wd_set_focus(s,w->id);
    } else if (q->operation==WD_MOVE || q->operation==WD_RESIZE ||
               q->operation==WD_FOCUS || q->operation==WD_CLOSE ||
               q->operation==WD_QUERY || q->operation==WD_MINIMIZE ||
               q->operation==WD_RESTORE || q->operation==WD_MAXIMIZE ||
               q->operation==WD_UNMAXIMIZE) {
        w=wd_find(s,q->id);
        if (!w) { r.verdict=WD_NOT_FOUND; return r; }
        if (w->owner!=principal) { r.verdict=WD_DENIED; return r; }
        if (q->operation==WD_MOVE) {
            if (w->maximized) return r;
            w->x=wd_clamp(q->x,w->width,s->screen_width);
            w->y=wd_clamp(q->y,w->height,s->screen_height);
        } else if (q->operation==WD_RESIZE) {
            if (w->maximized) return r;
            if (!q->width || !q->height ||
                q->width>s->screen_width || q->height>s->screen_height) return r;
            w->width=q->width; w->height=q->height;
            w->x=wd_clamp(w->x,w->width,s->screen_width);
            w->y=wd_clamp(w->y,w->height,s->screen_height);
        } else if (q->operation==WD_FOCUS) {
            if (w->minimized) return r;
            wd_set_focus(s,w->id);
        } else if (q->operation==WD_MINIMIZE) {
            if (!w->minimized) {
                w->minimized=1;
                wd_forget_focus(s,w->id);
                if (s->focus_id==w->id)
                    s->focus_id=s->focus_count ? s->focus_history[s->focus_count-1] : 0;
            }
        } else if (q->operation==WD_RESTORE) {
            w->minimized=0;
            wd_set_focus(s,w->id);
        } else if (q->operation==WD_MAXIMIZE) {
            if (!w->maximized) {
                w->saved_x=w->x; w->saved_y=w->y;
                w->saved_width=w->width; w->saved_height=w->height;
                w->maximized=1;
            }
            w->x=0; w->y=0;
            w->width=s->screen_width; w->height=s->screen_height;
            w->minimized=0;
            wd_set_focus(s,w->id);
        } else if (q->operation==WD_UNMAXIMIZE) {
            if (w->maximized) {
                w->x=w->saved_x; w->y=w->saved_y;
                w->width=w->saved_width; w->height=w->saved_height;
                w->maximized=0;
            }
            w->minimized=0;
            wd_set_focus(s,w->id);
        } else if (q->operation==WD_CLOSE) {
            wd_u32 was_focused=(s->focus_id==w->id);
            w->alive=0;
            wd_forget_focus(s,w->id);
            if (was_focused)
                s->focus_id=s->focus_count ? s->focus_history[s->focus_count-1] : 0;
            w=0;
        }
    } else {
        return r; /* shutdown gere par le service, pas par la politique */
    }
    r.verdict=WD_OK;
    r.id=(q->operation==WD_OPEN || q->operation==WD_FOCUS_NEXT) ? s->focus_id : q->id;
    wd_report(&r,s,w);
    return r;
}
#endif
