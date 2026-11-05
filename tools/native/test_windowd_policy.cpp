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
    return 0;
}
