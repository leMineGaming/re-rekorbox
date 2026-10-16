import os,struct,sys,collections
root=sys.argv[1]
def tags(b):
    o=struct.unpack('>I',b[4:8])[0]
    while o<len(b):
        tag=b[o:o+4].decode(); h,t=struct.unpack('>II',b[o+4:o+12]); yield tag,b[o:o+h],b[o+h:o+t]; o+=t
C=collections.Counter(); ex={}
for dp,_,fs in os.walk(root):
    for f in fs:
        if f!='ANLZ0000.DAT' and f!='ANLZ0000.EXT': continue
        b=open(os.path.join(dp,f),'rb').read()
        for tag,h,body in tags(b):
            if tag=='PQTZ':
                n=struct.unpack('>I',h[20:24])[0]; ent=[struct.unpack('>HHI',body[i*8:i*8+8]) for i in range(n)]
                if not ent: continue
                C['pqtz_files']+=1
                C['pqtz_beat_in_1..4']+= all(1<=e[0]<=4 for e in ent)
                C['pqtz_beat_cycles']+= all(ent[i+1][0]==ent[i][0]%4+1 for i in range(n-1))
                C['pqtz_time_increasing']+= all(ent[i+1][2]>ent[i][2] for i in range(n-1))
                C['pqtz_tempo_6000..25000']+= all(6000<=e[1]<=25000 for e in ent)
                C['pqtz_const_tempo']+= len({e[1] for e in ent})==1
                ex.setdefault('pqtz',ent[:5])
            if tag=='PPTH':
                n=struct.unpack('>I',h[12:16])[0]; s=body[:n].decode('utf-16-be')
                C['ppth_nul_terminated']+= s.endswith('\0'); C['ppth']+=1; ex.setdefault('ppth',s[:60])
            if tag=='PWAV':
                C['pwav']+=1; C['pwav_top3bits_le7']+=all((x>>5)<=7 for x in body); ex.setdefault('pwav',[ (x>>5,x&31) for x in body[:8]])
            if tag=='PVBR':
                ex.setdefault('pvbr',struct.unpack('>8I',body[:32])); C['pvbr_body_len_'+str(len(body))]+=1
            if tag=='PQT2':
                ex.setdefault('pqt2_hdr',h[12:].hex()); ex.setdefault('pqt2_body',body[:32].hex())
            if tag=='PSSI':
                ex.setdefault('pssi_hdr',h[12:].hex()); ex.setdefault('pssi_body',body[:48].hex())
            if tag=='PVB2':
                ex.setdefault('pvb2_body',body[:40].hex())
            if tag=='PWV4': ex.setdefault('pwv4',body[:24].hex())
            if tag=='PWV5': ex.setdefault('pwv5',body[:16].hex())
            if tag=='PWV3': ex.setdefault('pwv3',body[:16].hex())
print(dict(C))
for k,v in ex.items(): print(k,':',v)
