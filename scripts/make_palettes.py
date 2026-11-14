import math
def oklch(L,C,h):
    a=C*math.cos(math.radians(h)); b=C*math.sin(math.radians(h))
    l_=L+0.3963377774*a+0.2158037573*b; m_=L-0.1055613458*a-0.0638541728*b; s_=L-0.0894841775*a-1.2914855480*b
    l,m,s=l_**3,m_**3,s_**3
    r=4.0767416621*l-3.3077115913*m+0.2309699292*s
    g=-1.2684380046*l+2.6097574011*m-0.3413193965*s
    bb=-0.0041960863*l-0.7034186147*m+1.7076147010*s
    def enc(x):
        x=max(0,min(1,x)); return round(255*(12.92*x if x<=0.0031308 else 1.055*x**(1/2.4)-0.055))
    return (enc(r),enc(g),enc(bb))
def lum(c):
    def f(v):
        v/=255; return v/12.92 if v<=0.04045 else ((v+0.055)/1.055)**2.4
    r,g,b=map(f,c); return 0.2126*r+0.7152*g+0.0722*b
def cr(a,b):
    la,lb=sorted([lum(a),lum(b)],reverse=True); return (la+0.05)/(lb+0.05)
hx=lambda c:'0x%02X%02X%02X'%c
# name, hue, accent chroma light/dark, accent L light/dark
SK=[("Cherry",25,.16,.13,.50,.74),("Apricot",50,.14,.12,.52,.77),("Saffron",88,.12,.12,.53,.84),
    ("Forest",150,.12,.12,.48,.77),("Lagoon",195,.10,.10,.49,.77),("Cobalt",262,.17,.12,.47,.75),
    ("Lavender",300,.15,.12,.48,.76),("Blossom",350,.15,.12,.52,.78)]
out=[]
for name,h,cl,cd,al,ad in SK:
    for dark in (False,True):
        if not dark:
            p=dict(background=oklch(.982,.010,h),surface=oklch(.996,.005,h),ink=oklch(.20,.018,h),
                   muted=oklch(.50,.03,h),hairline=oklch(.905,.018,h),accent=oklch(al,cl,h),viewport=oklch(.83,.025,h))
            p['on_accent']=p['surface']
        else:
            p=dict(background=oklch(.165,.012,h),surface=oklch(.20,.015,h),ink=oklch(.935,.012,h),
                   muted=oklch(.69,.022,h),hairline=oklch(.27,.016,h),accent=oklch(ad,cd,h),viewport=oklch(.115,.008,h))
            p['on_accent']=oklch(.17,.02,h)
        checks=dict(acc_bg=cr(p['accent'],p['background']),onacc=cr(p['on_accent'],p['accent']),
                    muted=cr(p['muted'],p['background']),ink=cr(p['ink'],p['background']))
        bad=[k for k,v in checks.items() if v<4.5]
        print(f"{name:9s} {'dark ' if dark else 'light'} accent={hx(p['accent'])} "+" ".join(f"{k}={v:.1f}" for k,v in checks.items()), "FAIL "+",".join(bad) if bad else "")
        out.append((name,dark,p))
import json
json.dump([(n,d,{k:hx(v) for k,v in p.items()}) for n,d,p in out],open(__file__.replace('.py','.json'),'w'))
