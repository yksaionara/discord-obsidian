const fs=require('fs');const dir=process.argv[2];const seen=new Map();
for(const f of fs.readdirSync(dir)){const s=fs.readFileSync(dir+'/'+f,'utf8');if(!s.includes('super("discord_protos'))continue;
 for(const m of s.matchAll(/super\("(discord_protos[\w.]+)", \[([\s\S]*?)\]\)/g)){const name=m[1];if(seen.has(name))continue;
  const fields=[];for(const fm of m[2].matchAll(/\{\s*no: (\d+),\s*name: "(\w+)",([\s\S]*?)\n\s*\}/g)){const body=fm[3];
   const kind=(body.match(/kind: "(\w+)"/)||[])[1];const T=(body.match(/T: (\d+|\(\) => [\w.$]+|[\w.$]+)/)||[])[1];const rep=/repeat: [12]/.test(body)?'repeated ':'';const opt=/opt: !0/.test(body)?'optional ':'';
   const map=/kind: "map"/.test(body)?' map':'';fields.push(`  ${rep}${opt}${kind}${T?'<'+T+'>':''} ${fm[2]} = ${fm[1]};`)}
  seen.set(name,`message ${name} { // ${f}\n${fields.join('\n')}\n}`)}}
fs.writeFileSync(process.argv[3],[...seen.keys()].sort().map(k=>seen.get(k)).join('\n\n'));console.log(seen.size,'messages');
