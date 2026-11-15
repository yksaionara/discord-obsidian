// dump all TypeScript-compiled enums: (e=>{...})(...) form: v[v.NAME=val]="NAME" or v.NAME="str"
const fs=require('fs');const dir=process.argv[2];const out=[];const seen=new Set();
const re=/([A-Za-z_$][\w$]*)\[\1\.([A-Za-z_$][\w$]*)=(-?[\w.+\-*()<"' ]{1,40}?)\]="\2"/g;
for(const f of fs.readdirSync(dir)){const s=fs.readFileSync(dir+'/'+f,'utf8');if(s.length<3000||!s.includes(']="'))continue;
 let cur=null,last=-1;
 for(const m of s.matchAll(re)){
  if(!cur||m.index-last>200||m[1]!==cur.v){if(cur&&cur.m.length>1)out.push(cur);cur={v:m[1],f,at:m.index,m:[]}}
  cur.m.push([m[2],m[3]]);last=m.index+m[0].length}
 if(cur&&cur.m.length>1)out.push(cur)}
let txt='';for(const e of out){const key=e.m.map(x=>x.join('=')).join(',');if(seen.has(key))continue;seen.add(key);
 txt+=`## ${e.m[0][0]}… (${e.m.length}) ${e.f}@${e.at}\n`+e.m.map(x=>`${x[0]} = ${x[1]}`).join('\n')+'\n\n'}
fs.writeFileSync(process.argv[3],txt);console.log(seen.size,'enums');
