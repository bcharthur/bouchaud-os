// Real policy compiled from the SAME header as the ring3 service.
#include <cassert>
#include "../../userland/services/windowd/window_policy.h"

static wd_request request(wd_u32 op, wd_u32 id, wd_i32 x=0, wd_i32 y=0,
                          wd_u32 width=0, wd_u32 height=0)
{
    return {WD_VERSION,op,id,x,y,width,height};
}
int main()
{
    wd_state state;
    wd_init(&state,1280,720);
    auto q=request(WD_OPEN,0,-200,-100,640,400);
    wd_reply r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_OK && r.id!=0 && r.x==0 && r.y==0);
    assert(r.width==640 && r.height==400 && r.focus_id==r.id);
    wd_u32 first=r.id;
    q=request(WD_OPEN,0,40,50,100,100);
    r=wd_apply(&state,2,&q);
    assert(r.verdict==WD_OK && r.id!=first);
    wd_u32 other=r.id;
    q=request(WD_MOVE,other,100,100);
    r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_DENIED); // another owner can never move it
    q=request(WD_FOCUS,other);
    r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_DENIED); // or steal its focus
    q=request(WD_CLOSE,other);
    r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_DENIED); // or close it
    q=request(WD_FOCUS,first);
    r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_OK && r.focus_id==first);
    q=request(WD_MOVE,first,2000000000,2000000000);
    r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_OK && r.x==640 && r.y==320);
    q=request(WD_RESIZE,first,0,0,1281,720);
    r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_INVALID);
    q=request(WD_RESIZE,first,0,0,1280,720);
    r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_OK && r.x==0 && r.y==0);
    q=request(WD_CLOSE,first);
    r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_OK && r.focus_id==other);
    q=request(WD_QUERY,first);
    r=wd_apply(&state,1,&q);
    assert(r.verdict==WD_NOT_FOUND);
    for (int i=0;i<WD_MAX_WINDOWS-1;i++) {
        q=request(WD_OPEN,0,0,0,20,20);
        assert(wd_apply(&state,1,&q).verdict==WD_OK);
    }
    q=request(WD_OPEN,0,0,0,20,20);
    assert(wd_apply(&state,1,&q).verdict==WD_NO_SLOTS);
    q=request(WD_QUERY,other);
    q.version=99;
    assert(wd_apply(&state,2,&q).verdict==WD_INVALID);
    // Protection multi-principaux : un handle ne confere jamais les fenetres
    // d'autrui et une fenetre fermee ne peut pas etre reutilisee par ID.
    q=request(WD_QUERY,other);
    assert(wd_apply(&state,0,&q).verdict==WD_INVALID);
    assert(wd_apply(&state,1,&q).verdict==WD_DENIED);
    q=request(WD_RESIZE,other,0,0,1,1);
    assert(wd_apply(&state,1,&q).verdict==WD_DENIED);
    q=request(WD_CLOSE,other);
    assert(wd_apply(&state,2,&q).verdict==WD_OK);
    assert(wd_apply(&state,2,&q).verdict==WD_NOT_FOUND);
    q=request(WD_OPEN,0,0,0,20,20);
    assert(wd_apply(&state,3,&q).verdict==WD_OK);
    wd_u32 fresh=state.focus_id;
    assert(fresh!=first && fresh!=other);
    q=request(WD_MOVE,fresh,42,42);
    assert(wd_apply(&state,2,&q).verdict==WD_DENIED);
    // Une requete de taille invalide ne mute rien.
    q=request(WD_QUERY,fresh);
    wd_reply before=wd_apply(&state,3,&q);
    q=request(WD_RESIZE,fresh,0,0,0,10);
    assert(wd_apply(&state,3,&q).verdict==WD_INVALID);
    q=request(WD_QUERY,fresh);
    wd_reply after=wd_apply(&state,3,&q);
    assert(after.x==before.x && after.y==before.y &&
           after.width==before.width && after.height==before.height);
    // La fermeture restaure le dernier focus, pas le dernier ID cree.
    wd_state focus;
    wd_init(&focus,1280,720);
    q=request(WD_OPEN,0,0,0,100,100);
    wd_u32 a=wd_apply(&focus,1,&q).id;
    wd_u32 b=wd_apply(&focus,1,&q).id;
    wd_u32 c=wd_apply(&focus,1,&q).id;
    q=request(WD_FOCUS,a);
    assert(wd_apply(&focus,1,&q).focus_id==a);
    q=request(WD_FOCUS,b);
    assert(wd_apply(&focus,1,&q).focus_id==b);
    q=request(WD_CLOSE,b);
    assert(wd_apply(&focus,1,&q).focus_id==a);
    q=request(WD_CLOSE,a);
    assert(wd_apply(&focus,1,&q).focus_id==c);
    q=request(WD_CLOSE,c);
    assert(wd_apply(&focus,1,&q).focus_id==0);
    // Focus suivant: rotation entre les seules fenetres du principal.
    wd_state cycle;
    wd_init(&cycle,800,600);
    q=request(WD_FOCUS_NEXT,0);
    assert(wd_apply(&cycle,1,&q).verdict==WD_NOT_FOUND);
    q=request(WD_OPEN,0,0,0,80,80);
    wd_u32 own_a=wd_apply(&cycle,1,&q).id;
    wd_u32 foreign=wd_apply(&cycle,2,&q).id;
    wd_u32 own_b=wd_apply(&cycle,1,&q).id;
    q=request(WD_FOCUS_NEXT,0);
    r=wd_apply(&cycle,1,&q);
    assert(r.verdict==WD_OK && r.id==own_a && r.focus_id==own_a);
    r=wd_apply(&cycle,1,&q);
    assert(r.verdict==WD_OK && r.id==own_b);
    r=wd_apply(&cycle,2,&q);
    assert(r.verdict==WD_OK && r.id==foreign);
    q=request(WD_CLOSE,own_a);
    assert(wd_apply(&cycle,1,&q).verdict==WD_OK);
    q=request(WD_FOCUS_NEXT,0);
    assert(wd_apply(&cycle,1,&q).id==own_b);
    q=request(WD_CLOSE,own_b);
    assert(wd_apply(&cycle,1,&q).verdict==WD_OK);
    q=request(WD_FOCUS_NEXT,0);
    assert(wd_apply(&cycle,1,&q).verdict==WD_NOT_FOUND);
    // Teardown applicatif: fermer toutes les fenetres du principal sans
    // detruire celles des autres applications, puis restaurer leur focus.
    wd_state teardown;
    wd_init(&teardown,800,600);
    q=request(WD_OPEN,0,0,0,80,80);
    wd_u32 owned1=wd_apply(&teardown,1,&q).id;
    wd_u32 survivor=wd_apply(&teardown,2,&q).id;
    wd_u32 owned2=wd_apply(&teardown,1,&q).id;
    q=request(WD_CLOSE_OWNED,1);
    assert(wd_apply(&teardown,1,&q).verdict==WD_INVALID);
    q=request(WD_QUERY,owned1);
    assert(wd_apply(&teardown,1,&q).verdict==WD_OK);
    q=request(WD_CLOSE_OWNED,0);
    r=wd_apply(&teardown,1,&q);
    assert(r.verdict==WD_OK && r.focus_id==survivor);
    q=request(WD_QUERY,owned1);
    assert(wd_apply(&teardown,1,&q).verdict==WD_NOT_FOUND);
    q=request(WD_QUERY,owned2);
    assert(wd_apply(&teardown,1,&q).verdict==WD_NOT_FOUND);
    q=request(WD_QUERY,survivor);
    assert(wd_apply(&teardown,2,&q).verdict==WD_OK);
    q=request(WD_CLOSE_OWNED,0);
    assert(wd_apply(&teardown,1,&q).verdict==WD_OK);
    assert(wd_apply(&teardown,2,&q).focus_id==0);
    return 0;
}
