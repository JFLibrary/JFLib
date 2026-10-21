# Python reference of Neu färben (t_clone.s rc_px): CheckColor (flood.s) + channel shift
def isc(a,b): t=a*b+0x80; return ((t>>8)+t)>>8
def ch(c,s): return (c>>s)&255
def pm(c):
    a=c>>24; return (c&0xFF000000)|(isc(ch(c,16),a)<<16)|(isc(ch(c,8),a)<<8)|isc(ch(c,0),a)
def rc(p, old, new, tolpct, premul=True):
    tol=(min(tolpct,100)*255+50)//100; thr=tol*tol*4
    cmp=pm(old) if premul else old
    aA=cmp>>24; pa=p>>24; f=pa if premul else 255
    s=(aA-pa)**2
    for sh in (0,8,16):
        d=ch(cmp,sh)-isc(ch(p,sh),f); s+=((1+d*d)*aA)>>8
    if s>thr: return p
    r=p&0xFF000000
    for sh in (0,8,16):
        v=ch(p,sh)+ch(new,sh)-ch(old,sh); r|=max(0,min(255,v))<<sh
    return r
W=0xFFFFFFFF; RED=0xFFFF0000; GREY=0xFF808080; YEL=0xFFFFFF00; BLK=0xFF000000; BLUE=0xFF0000FF
print("left, sec=white -> blue, tol50")
for n,p in [("white",W),("red",RED),("grey",GREY),("yellow",YEL),("black",BLK)]:
    print(" ",n,"%08X"%rc(p,W,BLUE,50))
print("right: old=blue new=white tol50")
for n,p in [("blue",BLUE),("g->",rc(GREY,W,BLUE,50)),("red",RED),("black",BLK)]:
    print(" ",n,"%08X"%rc(p,BLUE,W,50))
print("once on red -> blue tol50")
for n,p in [("red",RED),("white",W),("grey",GREY),("yellow",YEL),("black",BLK),("blue",BLUE)]:
    print(" ",n,"%08X"%rc(p,RED,BLUE,50))
print("tol 0, sec white->blue")
for n,p in [("white",W),("fefefe",0xFFFEFEFE)]:
    print(" ",n,"%08X"%rc(p,W,BLUE,0))
print("alpha: red50 vs sec black50, tol30 -> blue")
print("  premul %08X straight %08X"%(rc(0x80FF0000,0x80000000,BLUE,30,True),rc(0x80FF0000,0x80000000,BLUE,30,False)))
print("  transparent 00000000 premul %08X straight %08X"%(rc(0x00000000,0x80000000,BLUE,30,True),rc(0x00000000,0x80000000,BLUE,30,False)))
