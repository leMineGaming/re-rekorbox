import struct,sys,mmap
def run(p):
    f=open(p,'rb'); m=mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ)
    pe=struct.unpack_from('<I',m,0x3c)[0]
    nsec=struct.unpack_from('<H',m,pe+6)[0]; ts=struct.unpack_from('<I',m,pe+8)[0]
    osz=struct.unpack_from('<H',m,pe+20)[0]; opt=pe+24
    magic=struct.unpack_from('<H',m,opt)[0]; dd=opt+(112 if magic==0x20b else 96)
    secs=[]; so=opt+osz
    for i in range(nsec):
        n,vs,va,rs,ra=struct.unpack_from('<8sIIII',m,so+i*40); secs.append((n.rstrip(b'\0').decode(),va,vs,ra,rs))
    def r2o(rva):
        for n,va,vs,ra,rs in secs:
            if va<=rva<va+max(vs,rs): return rva-va+ra
    def cstr(o): e=m.find(b'\0',o); return m[o:e].decode('latin1')
    print(p.split('/')[-1],'timestamp',ts,'sections',[(s[0],s[2]) for s in secs])
    irva,isz=struct.unpack_from('<II',m,dd+8)
    o=r2o(irva); out={}
    while True:
        ilt,_,_,name,iat=struct.unpack_from('<IIIII',m,o)
        if name==0: break
        dll=cstr(r2o(name)); fns=[]; t=r2o(ilt or iat)
        while True:
            v=struct.unpack_from('<Q' if magic==0x20b else '<I',m,t)[0]
            if v==0: break
            if v>>63 if magic==0x20b else v>>31: fns.append('#%d'%(v&0xffff))
            else: fns.append(cstr(r2o(v&0x7fffffff)+2))
            t+=8 if magic==0x20b else 4
        out[dll]=fns; o+=20
    for d,fns in out.items(): print(' %-40s %4d  %s'%(d,len(fns),', '.join(fns[:6])))
for p in sys.argv[1:]: run(p)
