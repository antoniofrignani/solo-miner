// Compiled after the unmodified production shader, reusing its SHA-256 helpers.
void hash_nonce(constant uint *prepared, uint nonce, thread uint *h, thread uint *w) {
    for(uint i=0;i<8;i++) h[i]=prepared[i];
    for(uint i=0;i<3;i++) w[i]=prepared[8+i];
    w[3]=sw(nonce);w[4]=0x80000000;for(uint i=5;i<15;i++)w[i]=0;w[15]=640;
    compress(h,w);
    for(uint i=0;i<8;i++)w[i]=h[i];w[8]=0x80000000;for(uint i=9;i<15;i++)w[i]=0;w[15]=256;
    init(h);compress(h,w);
}
void emit_nonce(thread uint *h, constant uint *target, constant uint *params,
                device uint *out, uint index) {
    if(index>=params[1])return;
    if(params[2]) { for(uint i=0;i<8;i++)out[index*8+i]=sw(h[i]);return; }
    bool match=true;
    for(int i=7;i>=0;i--){uint v=sw(h[i]);if(v<target[i])break;if(v>target[i]){match=false;break;}}
    if(match){uint slot=atomic_fetch_add_explicit((device atomic_uint*)out,1u,memory_order_relaxed);
        if(slot<params[3])out[slot+1]=params[0]+index;}
}
kernel void sha256d_two_sequential(constant uint *prepared [[buffer(0)]], constant uint *target [[buffer(1)]],
    constant uint *params [[buffer(2)]], device uint *out [[buffer(3)]], uint gid [[thread_position_in_grid]]) {
    uint index=gid*2;if(index>=params[1])return;
    uint wa[64],wb[64],ha[8],hb[8];
    hash_nonce(prepared,params[0]+index,ha,wa);
    if(index+1<params[1])hash_nonce(prepared,params[0]+index+1,hb,wb);
    emit_nonce(ha,target,params,out,index);
    if(index+1<params[1])emit_nonce(hb,target,params,out,index+1);
}
void compress_pair(thread uint *ha, thread uint *hb, thread uint *wa, thread uint *wb) {
    for(uint i=16;i<64;i++) {
        uint x=wa[i-15],y=wa[i-2];wa[i]=wa[i-16]+(rr(x,7)^rr(x,18)^(x>>3))+wa[i-7]+(rr(y,17)^rr(y,19)^(y>>10));
        x=wb[i-15];y=wb[i-2];wb[i]=wb[i-16]+(rr(x,7)^rr(x,18)^(x>>3))+wb[i-7]+(rr(y,17)^rr(y,19)^(y>>10));
    }
    uint a0=ha[0],b0=ha[1],c0=ha[2],d0=ha[3],e0=ha[4],f0=ha[5],g0=ha[6],h0=ha[7];
    uint a1=hb[0],b1=hb[1],c1=hb[2],d1=hb[3],e1=hb[4],f1=hb[5],g1=hb[6],h1=hb[7];
    for(uint i=0;i<64;i++) {
        uint t0=h0+(rr(e0,6)^rr(e0,11)^rr(e0,25))+((e0&f0)^((~e0)&g0))+K[i]+wa[i];
        uint t1=h1+(rr(e1,6)^rr(e1,11)^rr(e1,25))+((e1&f1)^((~e1)&g1))+K[i]+wb[i];
        uint u0=(rr(a0,2)^rr(a0,13)^rr(a0,22))+((a0&b0)^(a0&c0)^(b0&c0));
        uint u1=(rr(a1,2)^rr(a1,13)^rr(a1,22))+((a1&b1)^(a1&c1)^(b1&c1));
        h0=g0;g0=f0;f0=e0;e0=d0+t0;d0=c0;c0=b0;b0=a0;a0=t0+u0;
        h1=g1;g1=f1;f1=e1;e1=d1+t1;d1=c1;c1=b1;b1=a1;a1=t1+u1;
    }
    ha[0]+=a0;ha[1]+=b0;ha[2]+=c0;ha[3]+=d0;ha[4]+=e0;ha[5]+=f0;ha[6]+=g0;ha[7]+=h0;
    hb[0]+=a1;hb[1]+=b1;hb[2]+=c1;hb[3]+=d1;hb[4]+=e1;hb[5]+=f1;hb[6]+=g1;hb[7]+=h1;
}
kernel void sha256d_two_interleaved(constant uint *prepared [[buffer(0)]], constant uint *target [[buffer(1)]],
    constant uint *params [[buffer(2)]], device uint *out [[buffer(3)]], uint gid [[thread_position_in_grid]]) {
    uint index=gid*2;if(index>=params[1])return;
    uint wa[64],wb[64],ha[8],hb[8];
    for(uint i=0;i<8;i++){ha[i]=prepared[i];hb[i]=prepared[i];}
    for(uint i=0;i<3;i++){wa[i]=prepared[8+i];wb[i]=prepared[8+i];}
    wa[3]=sw(params[0]+index);wb[3]=sw(params[0]+index+(index+1<params[1]?1:0));
    wa[4]=wb[4]=0x80000000;for(uint i=5;i<15;i++){wa[i]=0;wb[i]=0;}wa[15]=wb[15]=640;
    compress_pair(ha,hb,wa,wb);
    for(uint i=0;i<8;i++){wa[i]=ha[i];wb[i]=hb[i];}wa[8]=wb[8]=0x80000000;
    for(uint i=9;i<15;i++){wa[i]=0;wb[i]=0;}wa[15]=wb[15]=256;
    init(ha);init(hb);compress_pair(ha,hb,wa,wb);
    emit_nonce(ha,target,params,out,index);emit_nonce(hb,target,params,out,index+1);
}
