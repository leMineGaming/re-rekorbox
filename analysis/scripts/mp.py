import struct,sys,os,collections
def dec(b,o):
    t=b[o]
    if t<=0x7f: return t,o+1
    if t>=0xe0: return t-256,o+1
    if 0x80<=t<=0x8f: return dmap(b,o+1,t&15)
    if 0x90<=t<=0x9f: return darr(b,o+1,t&15)
    if 0xa0<=t<=0xbf: n=t&31; return b[o+1:o+1+n].decode(),o+1+n
    if t==0xc0: return None,o+1
    if t in(0xc2,0xc3): return t==0xc3,o+1
    F={0xca:('>f',4),0xcb:('>d',8),0xcc:('>B',1),0xcd:('>H',2),0xce:('>I',4),0xcf:('>Q',8),0xd0:('>b',1),0xd1:('>h',2),0xd2:('>i',4),0xd3:('>q',8)}
    if t in F: f,n=F[t]; return struct.unpack_from(f,b,o+1)[0],o+1+n
    if t==0xd9: n=b[o+1]; return b[o+2:o+2+n].decode(),o+2+n
    if t==0xda: n=struct.unpack_from('>H',b,o+1)[0]; return b[o+3:o+3+n].decode(),o+3+n
    if t==0xdc: n=struct.unpack_from('>H',b,o+1)[0]; return darr(b,o+3,n)
    if t==0xdd: n=struct.unpack_from('>I',b,o+1)[0]; return darr(b,o+5,n)
    if t==0xde: n=struct.unpack_from('>H',b,o+1)[0]; return dmap(b,o+3,n)
    if t in(0xc4,0xc5,0xc6):
        k={0xc4:('>B',1),0xc5:('>H',2),0xc6:('>I',4)}[t]; n=struct.unpack_from(k[0],b,o+1)[0]; s=o+1+k[1]; return ('bin',b[s:s+n]),s+n
    raise ValueError('type %02x at %d'%(t,o))
def darr(b,o,n):
    r=[]
    for _ in range(n): v,o=dec(b,o); r.append(v)
    return r,o
def dmap(b,o,n):
    r={}
    for _ in range(n): k,o=dec(b,o); v,o=dec(b,o); r[k]=v
    return r,o
def shape(v):
    if isinstance(v,dict): return {k:shape(x) for k,x in v.items()}
    if isinstance(v,list): return 'list[%d] of %s'%(len(v), sorted({type(x).__name__ for x in v}))
    return type(v).__name__+('=%r'%v if not isinstance(v,(bytes,tuple)) else '')
if __name__=='__main__':
    root=sys.argv[1]; shapes=collections.Counter(); ok=0; bad=0; ex=None
    for d,_,fs in os.walk(root):
        for f in fs:
            if not f.endswith('.3EX'): continue
            b=open(os.path.join(d,f),'rb').read()
            try:
                v,o=dec(b,0); assert o==len(b); ok+=1
                s=shape(v); key=str({k:{kk:(vv.split('=')[0] if isinstance(vv,str) else vv) for kk,vv in s[k].items()} if isinstance(s[k],dict) else s[k] for k in s})
                import re; shapes[re.sub(r'list\[\d+\]','list[N]',key)]+=1
                if ex is None: ex=s
            except Exception as e: bad+=1; print('ERR',f,e)
    print('ok',ok,'bad',bad); print('example:',ex)
    for k,c in shapes.most_common(5): print(c,k)
