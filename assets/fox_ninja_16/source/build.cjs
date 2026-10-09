/* Native pixel authoring. Every draw operation writes whole pixels into a 16x16
 * grid. The imagegen concept is a visual reference only; it is never sampled,
 * resized, quantized, or used as the source of these game sprites. */
const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');
let PNG;
try { ({PNG} = require('pngjs')); } catch {
  ({PNG} = require('C:/Users/ASUS/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/pngjs'));
}
const ROOT = path.resolve(__dirname, '..');
const P = {
  '.': [0,0,0,0],
  k: [32,29,45,255],   // ink / eyes
  d: [47,48,71,255],   // cloth shadow
  b: [66,76,103,255],  // indigo cloth
  h: [105,123,150,255],// cloth highlight
  O: [188,72,44,255],  // fox shadow
  o: [242,125,49,255], // fox orange
  y: [255,177,76,255], // fox light
  c: [255,227,175,255],// muzzle / tail tip
  R: [141,39,61,255],  // headband shadow
  r: [235,65,69,255],  // headband / sash
  t: [255,121,101,255] // headband light
};
const keys = Object.keys(P);
function grid(w=16,h=16) { return Array.from({length:h},()=>Array(w).fill('.')); }
function put(g,x,y,c) {
  if (!Number.isInteger(x)||!Number.isInteger(y)) throw Error('Subpixel position');
  if (x<0||y<0||y>=g.length||x>=g[0].length) throw Error(`Clipped pixel ${x},${y}`);
  if (!(c in P)) throw Error(`Unknown palette entry ${c}`);
  g[y][x]=c;
}
function stamp(g,x,y,rows) { rows.forEach((r,j)=>[...r].forEach((c,i)=>{ if(c!=='.') put(g,x+i,y+j,c); })); }
function line(g,x0,y0,x1,y1,c) {
  let dx=Math.abs(x1-x0),sx=x0<x1?1:-1,dy=-Math.abs(y1-y0),sy=y0<y1?1:-1,err=dx+dy;
  while(true) {put(g,x0,y0,c); if(x0===x1&&y0===y1)break;const e=2*err;if(e>=dy){err+=dy;x0+=sx;}if(e<=dx){err+=dx;y0+=sy;}}
}
function limb(g,points,colour='b',end=null) {
  for(let j=1;j<points.length;j++){
    const [x0,y0]=points[j-1],[x1,y1]=points[j];
    line(g,x0-1,y0,x1-1,y1,'k');line(g,x0+1,y0,x1+1,y1,'k');
    line(g,x0,y0-1,x1,y1-1,'k');line(g,x0,y0+1,x1,y1+1,'k');
  }
  for(let j=1;j<points.length;j++)line(g,...points[j-1],...points[j],colour);
  if(end){const [x,y]=points.at(-1);put(g,x,y,end);}
}
const HEAD = [
  '..k...k..',
  '.kok.kok.',
  '.kcokoyk.',
  '.krrrrrk.',
  '.koookok.',
  '.kooockck',
  '..kcccck.',
  '...kkkk..'
];
const HEAD_LOW = [
  '..k..k...',
  '.kokkok..',
  '.krrrrrk.',
  '.koookok.',
  '.kooockck',
  '..kcccck.',
  '...kkkk..'
];
const TAILS = [
 ['..kk.','.kcck','kccok','koook','.kOOk','..kk.'],
 ['.kk..','kcck.','kcoOk','.koOk','..kOk','...k.'],
 ['.....','.kk..','kcckk','kcoOk','.kOOk','..kk.'],
 ['.....','.....','kk...','kcckk','.koOk','..kk.']
];
function tail(g,which=0,x=1,y=8){stamp(g,x,y,TAILS[which]);}
function ribbon(g,x,y,which=0){
  const rows=[['kk.k','krrk','.kRk'],['kk..','krrk','.kRk'],['.kkk','krrk','kRk.'],['....','krrk','.kRk']][which];
  stamp(g,x,y,rows);
}
function head(g,{x=5,y=1,blink=false,low=false}={}){
  stamp(g,x,y,low?HEAD_LOW:HEAD);
  if(blink){put(g,x+5,y+(low?3:4),'o');put(g,x+5,y+(low?4:5),'k');}
  put(g,x+3,y+(low?2:3),'t');
}
function torso(g,x=6,y=9){stamp(g,x,y,['.kkk.','kbhbk','kbbbk','.krrk']);}
function idle(i){
  const g=grid(),b=[0,0,1,1,1,0,0,0][i];
  tail(g,[0,1,1,0,2,2,0,1][i]);
  stamp(g,6,13,['kdk.kbk','kkk.kkk']);
  torso(g,6,9);
  stamp(g,10,10+b,['.k.','kok','.k.']);
  ribbon(g,2,4+b,[0,1,1,3,3,0,2,2][i]);
  head(g,{y:1+b,blink:i===4});
  return g;
}
function run(i){
  const g=grid(),bob=[0,1,0,-1,0,1,0,-1][i];
  const near=[[[9,12],[11,13],[12,14]],[[9,12],[10,13],[10,14]],[[9,12],[8,13],[7,14]],[[9,11],[7,12],[6,12]],[[9,12],[7,13],[5,14]],[[9,12],[8,13],[8,14]],[[9,12],[9,13],[11,14]],[[9,11],[10,12],[12,12]]][i];
  const far=[[[7,12],[5,13],[4,14]],[[7,12],[6,13],[6,14]],[[7,12],[8,12],[9,13]],[[7,11],[10,12],[11,12]],[[7,12],[10,13],[12,14]],[[7,12],[9,13],[9,14]],[[7,12],[7,13],[6,14]],[[7,11],[6,12],[5,12]]][i];
  tail(g,[2,0,1,1,2,3,0,1][i],1,7+bob);
  const grounded = pts => pts.map(([x,y])=>[x,y>=12?y-1:y]);
  limb(g,grounded(far),'d');
  limb(g,grounded(near),'b');
  torso(g,6,8+bob);
  if(i<4)limb(g,[[9,9+bob],[10,10+bob],[12,9+bob]],'b','o');
  else limb(g,[[9,9+bob],[8,10+bob],[6,10+bob]],'b','o');
  ribbon(g,2,3+bob,[0,2,0,1,2,0,1,0][i]);
  head(g,{x:6,y:1+bob});
  return g;
}
function dash(i){
  if(i===7)return idle(0);
  const g=grid();
  if(i===0){
    tail(g,2);stamp(g,5,12,['.kk.kk..','kbbkbbk.','kkk.kkkk']);
    torso(g,6,10);ribbon(g,2,6,3);head(g,{x:6,y:3,low:true});return g;
  }
  if(i===6){
    tail(g,2);limb(g,[[8,12],[6,12],[5,13]],'d');limb(g,[[9,12],[11,12],[12,13]],'b');
    torso(g,6,9);ribbon(g,2,5,0);head(g,{x:6,y:2,low:true});return g;
  }
  const dy=[0,1,0,0,1,0][i];
  // Tail and ties are swept into long horizontal clusters, inside the same cell.
  stamp(g,0,8+dy,['.kkkkkk.','kccoooOk','.kccooOk','..kkkkk.']);
  stamp(g,3,10+dy,['..kkkkkk..','kkddbhbbkk','kddbbbbbbk','.kkkrrrkk.']);
  const legs=[[],[[5,12],[4,13],[3,13]],[[6,12],[4,12],[2,12]],[[6,12],[5,13],[3,13]],[[6,12],[4,12],[2,12]],[[6,12],[5,13],[3,13]]][i];
  limb(g,legs,'d');
  stamp(g,2,6+dy,['kkkk..','krrrkk','.kRRrk']);
  head(g,{x:7,y:5+dy,low:true});
  put(g,12,12+dy,'o');
  return g;
}
function climb(i){
  const g=grid(),bob=[0,0,-1,-1,0,0,-1,-1][i];
  tail(g,[3,2,0,1,3,2,0,1][i],3,9+bob);
  // Far hand reaches, takes weight, and trades with the nearer hand.
  const farY=[6,4,2,3,6,7,7,6][i],nearY=[3,4,6,7,3,2,3,4][i];
  limb(g,[[10,9+bob],[12,7+bob],[14,farY]],'d','O');
  const nearLeg=[[[10,11],[11,13],[13,13]],[[10,11],[12,12],[13,11]],[[10,10],[12,10],[13,9]],[[10,10],[11,12],[13,12]],[[10,11],[11,13],[13,13]],[[10,11],[12,12],[13,11]],[[10,10],[12,10],[13,9]],[[10,10],[11,12],[13,12]]][i];
  limb(g,[[9,11+bob],[9,13+bob],[11,14+bob]],'d');
  limb(g,nearLeg,'b');
  torso(g,8,8+bob);
  ribbon(g,4,4+bob,[0,1,1,0,2,0,1,2][i]);
  head(g,{x:6,y:1+bob});
  limb(g,[[11,9+bob],[13,8+bob],[14,nearY]],'b','o');
  return g;
}
function mirror(g){return g.map(r=>[...r].reverse());}
function wallJump(i){
  if(i===0)return climb(3);
  const g=grid();
  if(i===1){
    tail(g,2,2,8);limb(g,[[8,11],[11,10],[14,10]],'d');limb(g,[[9,12],[12,13],[14,13]],'b');
    torso(g,6,8);ribbon(g,2,4,2);head(g,{x:5,y:1});limb(g,[[10,9],[12,8],[14,7]],'b','o');return g;
  }
  // Author the airborne right-facing silhouette, then mirror to face away
  // from the wall. Translation is supplied by the game, not baked into frames.
  const rise=[0,0,0,-1,-1,0,0,0][i];
  tail(g,[0,0,3,3,2,0,1,1][i],1,8+rise);
  const legs=[null,null,[[7,11],[5,12],[4,13]],[[7,11],[6,12],[7,13]],[[7,11],[6,12],[7,12]],[[7,12],[7,13],[6,14]],[[7,12],[6,13],[5,14]],[[7,12],[6,13],[5,14]]][i];
  limb(g,legs,'d');
  const front=[null,null,[[9,11],[11,12],[12,13]],[[9,11],[10,12],[11,12]],[[9,11],[11,11],[12,10]],[[9,11],[11,11],[12,12]],[[9,12],[11,12],[12,13]],[[9,12],[10,13],[10,14]]][i];
  limb(g,front,'b');
  torso(g,6,8+rise);
  ribbon(g,2,3+rise,[0,0,2,0,1,2,1,0][i]);
  head(g,{x:6,y:1+rise});
  limb(g,[[10,9+rise],[11,10+rise],[12,9+rise]],'b','o');
  return mirror(g);
}
const clips = [
 {name:'idle',build:idle,durations:[160,160,120,80,80,160,200,240],loop:true},
 {name:'run',build:run,durations:Array(8).fill(60),loop:true},
 {name:'dash',build:dash,durations:[60,40,40,40,40,40,60,80],loop:false,loopRange:[2,5]},
 {name:'wall_climb',build:climb,durations:Array(8).fill(100),loop:true},
 {name:'wall_jump',build:wallJump,durations:[60,40,60,80,100,100,120,120],loop:false}
];
function rgba(g,scale=1,bg=null){
  const w=g[0].length*scale,h=g.length*scale,out=Buffer.alloc(w*h*4);
  for(let y=0;y<h;y++)for(let x=0;x<w;x++){
    const c=g[Math.floor(y/scale)][Math.floor(x/scale)];
    const value=c==='.'&&bg?bg:P[c];
    out.set(value,(y*w+x)*4);
  }
  return {width:w,height:h,data:out};
}
function savePng(filename,g,scale=1,bg=null){
  fs.writeFileSync(path.join(ROOT,filename),PNG.sync.write(rgba(g,scale,bg),{colorType:6,bitDepth:8}));
}
function u16(n){const b=Buffer.alloc(2);b.writeUInt16LE(n);return b;}
// Small, dependency-free indexed GIF writer. Reset the LZW dictionary every
// 200 symbols, so all codes remain exactly 9 bits; the PNGs remain game assets.
function gif(filename,frames,delays,scale=8,bg=[24,28,43,255]){
  const w=frames[0][0].length*scale,h=frames[0].length*scale;
  const pal=Buffer.alloc(256*3);keys.forEach((key,i)=>pal.set((i?P[key]:bg).slice(0,3),i*3));
  const chunks=[Buffer.from('GIF89a'),u16(w),u16(h),Buffer.from([0xf7,0,0]),pal,Buffer.from([0x21,0xff,0x0b]),Buffer.from('NETSCAPE2.0'),Buffer.from([3,1,0,0,0])];
  frames.forEach((g,fi)=>{
    chunks.push(Buffer.from([0x21,0xf9,4,4]),u16(Math.max(2,Math.round(delays[fi]/10))),Buffer.from([0,0]),Buffer.from([0x2c]),u16(0),u16(0),u16(w),u16(h),Buffer.from([0,8]));
    const codes=[],indexes=[];
    for(let y=0;y<h;y++)for(let x=0;x<w;x++)indexes.push(keys.indexOf(g[Math.floor(y/scale)][Math.floor(x/scale)]));
    for(let s=0;s<indexes.length;s+=200){codes.push(256,...indexes.slice(s,s+200));}codes.push(257);
    const packed=Buffer.alloc(Math.ceil(codes.length*9/8));let bit=0;
    for(const code of codes){for(let k=0;k<9;k++,bit++)packed[bit>>3]|=((code>>k)&1)<<(bit&7);}
    for(let s=0;s<packed.length;s+=255){const chunk=packed.subarray(s,s+255);chunks.push(Buffer.from([chunk.length]),chunk);}chunks.push(Buffer.from([0]));
  });
  chunks.push(Buffer.from([0x3b]));fs.writeFileSync(path.join(ROOT,filename),Buffer.concat(chunks));
}
const atlas=grid(128,80),allFrames=[],frameMap={},tags=[];
for(let row=0;row<clips.length;row++){
  const clip=clips[row];clip.frames=Array.from({length:8},(_,i)=>clip.build(i));
  const strip=grid(128,16),leftStrip=grid(128,16);
  fs.mkdirSync(path.join(ROOT,'frames',clip.name),{recursive:true});
  for(let i=0;i<8;i++){
    const g=clip.frames[i],name=`${clip.name}_${String(i).padStart(2,'0')}`;
    stamp(atlas,i*16,row*16,g.map(r=>r.join('')));stamp(strip,i*16,0,g.map(r=>r.join('')));stamp(leftStrip,i*16,0,mirror(g).map(r=>r.join('')));
    savePng(`frames/${clip.name}/${name}.png`,g);allFrames.push(g);
    frameMap[name]={frame:{x:i*16,y:row*16,w:16,h:16},rotated:false,trimmed:false,spriteSourceSize:{x:0,y:0,w:16,h:16},sourceSize:{w:16,h:16},duration:clip.durations[i]};
  }
  savePng(`strips/${clip.name}.png`,strip);savePng(`strips/${clip.name}_mirrored.png`,leftStrip);
  savePng(`previews/${clip.name}-8x.png`,strip,8);
  gif(`previews/${clip.name}.gif`,clip.frames,clip.durations.map((d,i)=>!clip.loop&&i===7?d+600:d));
  tags.push({name:clip.name,from:row*8,to:row*8+7,direction:'forward'});
}
savePng('fox-ninja.png',atlas);savePng('previews/fox-ninja-sheet-8x.png',atlas,8);
const metadata={frames:frameMap,meta:{app:'Native 16x16 pixel authoring',version:'1.0',image:'fox-ninja.png',format:'RGBA8888',size:{w:128,h:80},scale:'1',frameTags:tags},game:{cell:{w:16,h:16},pivot:{x:8,y:15},recommendedCollider:{x:5,y:3,w:6,h:12},import:{filter:'nearest',mipmaps:false,compression:'lossless',padding:0,margin:0,pixelsPerUnit:16},animations:Object.fromEntries(clips.map(c=>[c.name,{frames:8,loop:c.loop,durationsMs:c.durations,...(c.loopRange?{holdLoopInclusive:c.loopRange}:{}),...(c.name==='wall_jump'?{direction:'Push off a wall on the right; face left from frame 2'}:{direction:c.name==='wall_climb'?'Wall on the right':'Right'})}]))}};
fs.writeFileSync(path.join(ROOT,'fox-ninja.json'),JSON.stringify(metadata,null,2)+'\n');
fs.writeFileSync(path.join(ROOT,'source','pixels.json'),JSON.stringify({palette:P,animations:Object.fromEntries(clips.map(c=>[c.name,c.frames.map(g=>g.map(r=>r.join('')))]))},null,2)+'\n');
fs.writeFileSync(path.join(ROOT,'palette.hex'),keys.slice(1).map(k=>P[k].slice(0,3).map(n=>n.toString(16).padStart(2,'0')).join('')).join('\n')+'\n');

// Aseprite document: 40 native 16x16 animation cels, one layer, five tags.
function u32(n){const b=Buffer.alloc(4);b.writeUInt32LE(n);return b;}
function aseString(s){const b=Buffer.from(s);return Buffer.concat([u16(b.length),b]);}
function chunk(type,data){return Buffer.concat([u32(data.length+6),u16(type),data]);}
const aseFrames=[];
for(let n=0;n<allFrames.length;n++){
  const chunks=[];
  if(n===0){
    chunks.push(chunk(0x2004,Buffer.concat([u16(3),u16(0),u16(0),u16(16),u16(16),u16(0),Buffer.from([255,0,0,0]),aseString('Fox ninja')])));
    chunks.push(chunk(0x2019,Buffer.concat([u32(keys.length),u32(0),u32(keys.length-1),Buffer.alloc(8),...keys.map(k=>Buffer.concat([u16(0),Buffer.from(P[k])]))])));
    chunks.push(chunk(0x2018,Buffer.concat([u16(tags.length),Buffer.alloc(8),...tags.map((t,j)=>Buffer.concat([u16(t.from),u16(t.to),Buffer.from([0]),u16(clips[j].loop?0:1),Buffer.alloc(6),Buffer.from([242,125,49,0]),aseString(t.name)]))])));
  }
  chunks.push(chunk(0x2005,Buffer.concat([u16(0),u16(0),u16(0),Buffer.from([255]),u16(2),u16(0),Buffer.alloc(5),u16(16),u16(16),zlib.deflateSync(rgba(allFrames[n]).data)])));
  const payload=Buffer.concat(chunks),header=Buffer.alloc(16);header.writeUInt32LE(16+payload.length,0);header.writeUInt16LE(0xf1fa,4);header.writeUInt16LE(chunks.length,6);header.writeUInt16LE(clips[Math.floor(n/8)].durations[n%8],8);aseFrames.push(Buffer.concat([header,payload]));
}
const aseHeader=Buffer.alloc(128);aseHeader.writeUInt32LE(128+aseFrames.reduce((s,b)=>s+b.length,0),0);aseHeader.writeUInt16LE(0xa5e0,4);aseHeader.writeUInt16LE(40,6);aseHeader.writeUInt16LE(16,8);aseHeader.writeUInt16LE(16,10);aseHeader.writeUInt16LE(32,12);aseHeader.writeUInt32LE(1,14);aseHeader.writeUInt16LE(100,18);aseHeader.writeUInt16LE(keys.length,32);aseHeader[34]=1;aseHeader[35]=1;aseHeader.writeUInt16LE(16,40);aseHeader.writeUInt16LE(16,42);
fs.writeFileSync(path.join(ROOT,'fox-ninja.aseprite'),Buffer.concat([aseHeader,...aseFrames]));
console.log('Built 40 native 16x16 frames, a 128x80 atlas, strips, previews, metadata, and Aseprite document.');

// A compact animated contact sheet, rendered from the original pixel arrays.
const FONT = {
 A:['010','101','111','101','101'],B:['110','101','110','101','110'],C:['011','100','100','100','011'],
 D:['110','101','101','101','110'],E:['111','100','110','100','111'],F:['111','100','110','100','100'],
 G:['011','100','101','101','011'],H:['101','101','111','101','101'],I:['111','010','010','010','111'],
 J:['001','001','001','101','010'],K:['101','101','110','101','101'],L:['100','100','100','100','111'],
 M:['101','111','111','101','101'],N:['101','111','111','111','101'],O:['010','101','101','101','010'],
 P:['110','101','110','100','100'],Q:['010','101','101','111','011'],R:['110','101','110','101','101'],
 S:['011','100','010','001','110'],T:['111','010','010','010','010'],U:['101','101','101','101','111'],
 V:['101','101','101','101','010'],W:['101','101','111','111','101'],X:['101','101','010','101','101'],
 Y:['101','101','010','010','010'],Z:['111','001','010','100','111'],
 '0':['111','101','101','101','111'],'1':['010','110','010','010','111'],'2':['110','001','010','100','111'],
 '3':['110','001','010','001','110'],'4':['101','101','111','001','001'],'5':['111','100','110','001','110'],
 '6':['011','100','110','101','010'],'7':['111','001','010','010','010'],'8':['010','101','010','101','010'],
 '9':['010','101','011','001','110'],'-':['000','000','111','000','000'],'/':['001','001','010','100','100'],
 ' ':['000','000','000','000','000']
};
function label(g,text,x,y,c='c'){
  for(const letter of text.toUpperCase()){
    const glyph=FONT[letter]||FONT[' '];glyph.forEach((r,j)=>[...r].forEach((v,i)=>{if(v==='1')put(g,x+i,y+j,c);}));x+=4;
  }
}
function frameAt(clip,time){
  let t=time%(clip.loop?clip.durations.reduce((a,b)=>a+b,0):1200);
  for(let i=0;i<8;i++){if(t<clip.durations[i])return i;t-=clip.durations[i];}return 7;
}
const overviewFrames=[];
for(let t=0;t<2400;t+=20){
  const g=grid(220,42);
  clips.forEach((clip,j)=>{
    const title=clip.name.replaceAll('_',' '),start=j*44,fi=frameAt(clip,t);
    label(g,title,start+Math.floor((44-(title.length*4-1))/2),3);
    stamp(g,start+14,14,clip.frames[fi].map(r=>r.join('')));
    if(j<3)line(g,start+7,30,start+37,30,'b');
    if(j===3)line(g,start+30,12,start+30,31,'b');
    if(j===4&&fi<2)line(g,start+30,12,start+30,31,'b');
    label(g,`${fi+1}/8`,start+17,34,'h');
  });
  overviewFrames.push(g);
}
gif('previews/animations.gif',overviewFrames,overviewFrames.map(()=>20),4);
savePng('previews/animations.png',overviewFrames[12],4,[24,28,43,255]);
// Labelled contact sheet; display pixels are integer multiples of source pixels.
const contact=grid(174,126);
label(contact,'FOX NINJA',6,5,'c');label(contact,'16X16 / 40 FRAMES',6,14,'h');
clips.forEach((clip,row)=>{
  const y=28+row*18;
  const words=clip.name.split('_');words.forEach((word,i)=>label(contact,word,5,y+3+i*6,'h'));
  clip.frames.forEach((g,col)=>stamp(contact,29+col*18,y,g.map(r=>r.join(''))));
});
label(contact,'NATIVE PNG / ALPHA / 5 ANIMATIONS',6,120,'h');
savePng('previews/contact-sheet.png',contact,6,[24,28,43,255]);

const dataUri='data:image/png;base64,'+fs.readFileSync(path.join(ROOT,'fox-ninja.png')).toString('base64');
const clipInfo=clips.map(({name,durations,loop},row)=>({name,durations,loop,row}));
const html=`<!doctype html>
<html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Fox Ninja · Native 16×16</title>
<style>
*{box-sizing:border-box}body{margin:0;background:#111624;color:#f4e9d7;font:15px/1.55 system-ui,sans-serif}main{max-width:1280px;margin:auto;padding:48px 32px}header{display:flex;justify-content:space-between;align-items:center;gap:30px;margin-bottom:28px}.eyebrow{color:#ffac68;font:700 11px/1.4 ui-monospace,monospace;letter-spacing:3px}h1{font-size:clamp(32px,5vw,56px);letter-spacing:-2px;line-height:1.05;margin:12px 0}p{color:#a9b3c9;margin:10px 0}.tag{border:1px solid #3a445b;padding:8px 14px;border-radius:30px;white-space:nowrap;font-family:ui-monospace,monospace;color:#ffd99d}.controls{display:flex;align-items:center;gap:20px;flex-wrap:wrap;border-block:1px solid #2e384f;padding:16px 0;margin-bottom:24px}button,select,a.download{font:inherit;border:1px solid #424e68;border-radius:6px;background:#242e44;color:#f4e9d7;padding:8px 14px;cursor:pointer}button:hover,a.download:hover{border-color:#ffac68}label{display:flex;align-items:center;gap:8px;color:#a9b3c9}input{accent-color:#fa9659}.cards{display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:14px}.card{border:1px solid #303b51;border-radius:12px;overflow:hidden;background:#1a2233}.card h2{font-size:15px;margin:0}.card .top{padding:16px;display:flex;justify-content:space-between;align-items:center;gap:8px}.badge{font:10px ui-monospace,monospace;color:#9facc4}.stage{height:180px;display:flex;justify-content:center;align-items:center;background:#202b40}.stage canvas{width:160px;height:160px;image-rendering:pixelated}.bottom{padding:12px 16px}.counter{font:11px ui-monospace,monospace;color:#d0a379}.description{min-height:40px;font-size:12px;margin:8px 0;color:#a9b3c9}.native{display:flex;align-items:center;gap:10px;border-top:1px solid #35415a;padding-top:10px;margin-top:10px;font-size:10px;color:#8695b1}.native canvas{width:16px;height:16px;image-rendering:pixelated}.sheet{margin-top:40px;border-top:1px solid #303b51;padding-top:24px}.sheet h2{font-size:20px;margin:0 0 6px}.sheet img{display:block;image-rendering:pixelated;width:768px;max-width:100%;background-color:#1a2437;background-image:conic-gradient(#253047 25%,transparent 0 50%,#253047 0 75%,transparent 0);background-size:24px 24px;border:1px solid #35415a;border-radius:8px;margin:20px 0}.links{display:flex;gap:12px;flex-wrap:wrap}a{color:#ffbb83}.foot{font-size:12px;margin-top:25px} @media(max-width:1000px){.cards{grid-template-columns:repeat(3,1fr)}header{align-items:flex-start}}@media(max-width:620px){main{padding:26px 18px}.cards{grid-template-columns:repeat(2,1fr)}header{display:block}.tag{display:inline-block;margin-top:10px}.controls{gap:12px}}
</style><main><header><div><div class="eyebrow">PRECISION PLATFORMER · CHARACTER SET</div><h1>Fox Ninja</h1><p>Orange fur. Indigo cloth. A red headband. Every pixel has a job.</p></div><div class="tag">16 × 16 native pixels</div></header>
<div class="controls"><button id="play" type="button">Pause</button><label>Speed <select id="speed"><option value="0.5">0.5×</option><option value="1" selected>1×</option><option value="2">2×</option></select></label><label><input type="checkbox" id="flip"> Mirror</label><label><input type="checkbox" id="grid"> Pixel grid</label><label><input type="checkbox" id="light"> Light background</label><span style="margin-left:auto;color:#8e9bb6;font-size:12px">40 frames · 5 animations · transparent PNG</span></div>
<section class="cards" aria-label="Animation previews"></section>
<section class="sheet"><h2>The native sprite atlas</h2><p>128 × 80 PNG · 8 columns × 5 rows · 16 × 16 cells · no margins or spacing.</p><img src="${dataUri}" alt="Fox ninja sprite atlas. Rows: idle, run, dash, wall climb and wall jump."><div class="links"><a class="download" href="fox-ninja.png" download>PNG atlas</a><a class="download" href="fox-ninja.aseprite" download>Editable Aseprite</a><a class="download" href="fox-ninja.json" download>Animation data</a><a class="download" href="previews/animations.gif" download>Animated preview</a></div><p class="foot">Use nearest-neighbor filtering with mipmaps off. Ground pivot: (8, 15). Wall actions start against a wall on the right; mirror for the opposite side. Dash and wall jump replay here with a short pause for inspection.</p><p class="foot">Visual inspiration: <a href="https://pixelfrog-assets.itch.io/pixel-adventure-1">Pixel Frog's Pixel Adventure</a>. Original fox design: built-in ImageGen concept, then authored directly on the native pixel grid. The high-resolution concept is kept separately in the reference folder.</p></section>
</main><script>
const clips=${JSON.stringify(clipInfo)},atlas=new Image();atlas.src=${JSON.stringify(dataUri)};
const descriptions=['A soft breath, a tail swish, and a blink.','Eight stride poses with alternating feet.','Anticipation, launch, hold, and recovery.','Alternating grips and tucked knees.','Compress, push off, turn, and rise.'];
const cards=clips.map((clip,i)=>{const el=document.createElement('article');el.className='card';el.innerHTML='<div class="top"><h2>'+clip.name.replaceAll('_',' ').replace(/^./,s=>s.toUpperCase())+'</h2><span class="badge">'+(clip.loop?'LOOP':'ONE SHOT')+'</span></div><div class="stage"><canvas width="160" height="160" aria-label="'+clip.name+' enlarged animation"></canvas></div><div class="bottom"><div class="counter">FRAME 01 / 08</div><p class="description">'+descriptions[i]+'</p><div class="native"><canvas width="16" height="16" aria-label="'+clip.name+' at native resolution"></canvas><span>ACTUAL SIZE · 1×</span></div></div>';document.querySelector('.cards').append(el);return {el,ctx:el.querySelector('canvas').getContext('2d'),native:el.querySelector('.native canvas').getContext('2d')};});
let playing=true,time=0,last=0;const play=document.getElementById('play');play.onclick=()=>{playing=!playing;play.textContent=playing?'Pause':'Play';};
function at(c,t){const total=c.durations.reduce((a,b)=>a+b,0);let n=t%(c.loop?total:1200);for(let i=0;i<8;i++){if(n<c.durations[i])return i;n-=c.durations[i];}return 7;}
function draw(now){if(last&&playing)time+=(now-last)*Number(document.getElementById('speed').value);last=now;const flip=document.getElementById('flip').checked,grid=document.getElementById('grid').checked,light=document.getElementById('light').checked;
cards.forEach((card,j)=>{const clip=clips[j],f=at(clip,time),ctx=card.ctx;ctx.imageSmoothingEnabled=false;ctx.fillStyle=light?'#ece6d4':'#202b40';ctx.fillRect(0,0,160,160);ctx.strokeStyle=light?'#c8bfa8':'#3c4c68';ctx.lineWidth=1;ctx.beginPath();if(j<3){ctx.moveTo(8,144.5);ctx.lineTo(152,144.5);}if(j===3||(j===4&&f<2)){ctx.moveTo(flip?15.5:144.5,8);ctx.lineTo(flip?15.5:144.5,152);}ctx.stroke();ctx.save();if(flip){ctx.translate(160,0);ctx.scale(-1,1);}if(atlas.complete&&atlas.naturalWidth)ctx.drawImage(atlas,f*16,j*16,16,16,16,24,128,128);ctx.restore();if(grid){ctx.strokeStyle=light?'#4442':'#fff1';ctx.beginPath();for(let i=0;i<=16;i++){ctx.moveTo(16+i*8+.5,24);ctx.lineTo(16+i*8+.5,152);ctx.moveTo(16,24+i*8+.5);ctx.lineTo(144,24+i*8+.5);}ctx.stroke();}card.native.clearRect(0,0,16,16);card.native.imageSmoothingEnabled=false;card.native.save();if(flip){card.native.translate(16,0);card.native.scale(-1,1);}if(atlas.complete&&atlas.naturalWidth)card.native.drawImage(atlas,f*16,j*16,16,16,0,0,16,16);card.native.restore();card.el.querySelector('.counter').textContent='FRAME '+String(f+1).padStart(2,'0')+' / 08';});requestAnimationFrame(draw);}
requestAnimationFrame(draw);
</script></html>`;
fs.writeFileSync(path.join(ROOT,'preview.html'),html);
console.log('Saved animated overview, labelled contact sheet, and offline interactive preview.');
// Lossless GIF encoding optimisation only. No scaling, resampling, or redraw.
let sharp;
try { sharp=require('sharp'); } catch {
  try { sharp=require('C:/Users/ASUS/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/sharp'); } catch {}
}
if(sharp)Promise.all(fs.readdirSync(path.join(ROOT,'previews')).filter(n=>n.endsWith('.gif')).map(async n=>{
  const file=path.join(ROOT,'previews',n),bytes=fs.readFileSync(file);
  const out=await sharp(bytes,{animated:true}).gif({reuse:true,effort:7,dither:0,interFrameMaxError:0,interPaletteMaxError:0,keepDuplicateFrames:true}).toBuffer();
  fs.writeFileSync(file,out);
})).then(()=>console.log('Optimised GIF previews without changing pixels or timings.')).catch(e=>{console.error(e);process.exitCode=1;});
