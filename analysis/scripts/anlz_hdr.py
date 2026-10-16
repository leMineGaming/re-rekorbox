import os,struct,sys,collections
root=sys.argv[1]
fields=collections.defaultdict(lambda: collections.defaultdict(collections.Counter))
rel=collections.defaultdict(collections.Counter)
for dp,_,fs in os.walk(root):
    for f in fs:
        if not f.startswith('ANLZ') or f.endswith('3EX'): continue
        ext=f[-3:]; b=open(os.path.join(dp,f),'rb').read(); o=28; idx=collections.Counter()
        while o<len(b):
            tag=b[o:o+4].decode(); h,t=struct.unpack('>II',b[o+4:o+12]); hdr=b[o+12:o+h]; body_len=t-h
            key=ext+':'+tag+'#%d'%idx[tag]; idx[tag]+=1
            # header remainder as list of u32 (or u16 when odd)
            words=[struct.unpack('>I',hdr[i:i+4])[0] for i in range(0,len(hdr)-len(hdr)%4,4)]
            if len(hdr)%4: words.append('tail:'+hdr[len(hdr)-len(hdr)%4:].hex())
            for i,w in enumerate(words): fields[key][i][w]+=1
            # relation tests: word == body_len / n
            for i,w in enumerate(words):
                if isinstance(w,int) and w>0 and body_len>0 and body_len%w==0: rel[key][(i,'body/w=%d'%(body_len//w))]+=1
                if isinstance(w,int) and w==body_len: rel[key][(i,'==body_len')]+=1
            o+=t
for key in sorted(fields):
    print(key)
    for i in sorted(fields[key]):
        c=fields[key][i]; top=c.most_common(4)
        print('   w%d distinct=%d top=%s'%(i,len(c),top))
    r=[(k,v) for k,v in rel[key].most_common(6)]
    print('   rel:',r)
