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
// prepared: SHA-256 midstate and the last three fixed header words.
// params: nonce start, count, diagnostic output, result capacity.
kernel void sha256d_scan(constant uint *prepared [[buffer(0)]], constant uint *target [[buffer(1)]],
    constant uint *params [[buffer(2)]], device uint *out [[buffer(3)]], uint gid [[thread_position_in_grid]]) {
    if(gid>=params[1]) return;
    uint w[64],h[8];
    #pragma unroll
    for(uint i=0;i<8;i++) h[i]=prepared[i];
    for(uint i=0;i<3;i++) w[i]=prepared[8+i];
    w[3]=sw(params[0]+gid);w[4]=0x80000000;for(uint i=5;i<15;i++) w[i]=0;w[15]=640;
    compress(h,w);
    for(uint i=0;i<8;i++) w[i]=h[i];w[8]=0x80000000;for(uint i=9;i<15;i++)w[i]=0;w[15]=256;
    init(h);compress(h,w);
    if(params[2]) { for(uint i=0;i<8;i++) out[gid*8+i]=sw(h[i]); }
    else {
        bool match=true;
        for(int i=7;i>=0;i--){uint v=sw(h[i]);if(v<target[i])break;if(v>target[i]){match=false;break;}}
        if(match) {
            uint slot=atomic_fetch_add_explicit((device atomic_uint*)out,1u,memory_order_relaxed);
            if(slot<params[3]) out[slot+1]=params[0]+gid;
        }
    }
}
