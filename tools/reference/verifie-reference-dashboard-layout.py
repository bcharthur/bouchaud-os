#!/usr/bin/env python3
import sys
def r(x,y,w,h): return x,y,x+w,y+h
def ov(a,b): return not(a[2]<=b[0] or b[2]<=a[0] or a[3]<=b[1] or b[3]<=a[1])
def check(w,h):
    if w<720 or h<520: return "tiny"
    if w<1024 or h<760:
        m,g=28,12; cw=w-2*m; mw=(cw-g)//2; lw=cw*58//100
        boxes=[r(m,86,cw,92),r(m,190,mw,104),r(m+mw+g,190,mw,104),r(m,306,lw,176),r(m+lw+g,306,cw-lw-g,176),r(m,h-92,cw,56)]
        mode="compact"
    else:
        m,g=60,16; cw=w-2*m; card=(cw-2*g)//3; lw=cw*3//5
        boxes=[r(m,138,card,154),r(m+card+g,138,card,154),r(m+2*(card+g),138,card,154),r(m,312,lw,310),r(m+lw+g,312,cw-lw-g,310),r(m,h-116,cw,62)]
        mode="wide"
    for i,a in enumerate(boxes):
        if a[0]<0 or a[1]<0 or a[2]>w or a[3]>h: raise AssertionError((w,h,"bounds",a))
        for b in boxes[i+1:]:
            if ov(a,b): raise AssertionError((w,h,"overlap",a,b))
    return mode
for w,h,e in [(640,480,"tiny"),(800,600,"compact"),(1024,768,"wide"),(1280,720,"compact"),(1280,800,"wide")]:
    got=check(w,h)
    if got!=e: print(f"ECHEC {w}x{h}: {got} != {e}",file=sys.stderr); raise SystemExit(1)
    print(f"LAYOUT_OK {w}x{h} profile={got}")
print("REFERENCE_DASHBOARD_LAYOUT_OK")
