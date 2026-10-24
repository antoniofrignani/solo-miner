#include <metal_stdlib>
using namespace metal;
constant uint K[64] = {
0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2};
uint rr(uint v,uint n){return (v>>n)|(v<<(32-n));}
uint sw(uint x){return ((x&255)<<24)|((x&65280)<<8)|((x>>8)&65280)|(x>>24);}
void init(thread uint *h) { h[0]=0x6a09e667;h[1]=0xbb67ae85;h[2]=0x3c6ef372;h[3]=0xa54ff53a;h[4]=0x510e527f;h[5]=0x9b05688c;h[6]=0x1f83d9ab;h[7]=0x5be0cd19; }
void compress(thread uint *h, thread uint *w) {
    for(uint i=16;i<64;i++){uint a=w[i-15],b=w[i-2];w[i]=w[i-16]+(rr(a,7)^rr(a,18)^(a>>3))+w[i-7]+(rr(b,17)^rr(b,19)^(b>>10));}
    uint a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],v=h[7];
    for(uint i=0;i<64;i++){uint t=v+(rr(e,6)^rr(e,11)^rr(e,25))+((e&f)^((~e)&g))+K[i]+w[i];uint u=(rr(a,2)^rr(a,13)^rr(a,22))+((a&b)^(a&c)^(b&c));v=g;g=f;f=e;e=d+t;d=c;c=b;b=a;a=t+u;}
    h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=v;
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
