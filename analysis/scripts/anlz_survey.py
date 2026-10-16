import os,struct,sys,collections
root=sys.argv[1]
stats=collections.defaultdict(lambda: collections.Counter())
order=collections.Counter(); errors=collections.Counter(); files=0
hdrinfo=collections.defaultdict(set); minmax={}
for dp,_,fs in os.walk(root):
    for f in fs:
        if not f.startswith('ANLZ'): continue
        ext=f.rsplit('.',1)[1]; b=open(os.path.join(dp,f),'rb').read(); files+=1
        if b[:4]!=b'PMAI': errors[ext+':nomagic']+=1; continue
        hl,tl=struct.unpack('>II',b[4:12])
        if tl!=len(b): errors[ext+':pmai_len!=filesize']+=1
        hdrinfo[ext+':PMAI_hl'].add(hl); hdrinfo[ext+':PMAI_rest'].add(b[12:hl].hex())
        o=hl; seq=[]
        while o<len(b):
            if o+12>len(b): errors[ext+':trailing']+=1; break
            tag=b[o:o+4].decode('latin1'); h,t=struct.unpack('>II',b[o+4:o+12])
            if t<h or o+t>len(b): errors[ext+':badlen:'+tag]+=1; break
            seq.append(tag); stats[ext][tag]+=1; hdrinfo[ext+':'+tag+':hl'].add(h)
            k=ext+':'+tag; lo,hi=minmax.get(k,(1<<40,0)); minmax[k]=(min(lo,t),max(hi,t))
            o+=t
        order[ext+' '+' '.join(seq)]+=1
print('files',files,'errors',dict(errors))
for ext in sorted(stats): print(ext, dict(stats[ext]))
print('-- header lengths / total-length range')
for k in sorted(hdrinfo):
    v=hdrinfo[k]
    if k.endswith('_rest'): print(k, list(v)[:4], 'variants',len(v))
    else: print(k, sorted(v), minmax.get(k[:-3]) if k.endswith(':hl') else '')
print('-- tag orders (top)')
for s,c in order.most_common(12): print(c,s)
