// Lists every lazily loaded chunk of Discord's web client.
// usage: node chunks.js <dir with web.*.js> <out.txt>
// The webpack runtime in web.*.js names a chunk two ways: a few as
// `if(ID===e)return""+e+".HASH.js"`, the rest from one `({ID:"HASH",...})[e]+".js"` map,
// whose file is just HASH.js.
const fs = require('fs'), path = require('path');
const [dir, out] = process.argv.slice(2);
const runtime = fs.readdirSync(dir).find(n => n.startsWith('web.'));
const s = fs.readFileSync(path.join(dir, runtime), 'utf8');
const start = s.indexOf('.u=e=>{');
const seg = s.slice(start, s.indexOf('[e]+".js"},', start));
const files = [];
for (const m of seg.matchAll(/if\((\d+)===e\)return""\+e\+"(\.[0-9a-f]+\.js)"/g)) files.push(m[1] + m[2]);
for (const m of seg.slice(seg.lastIndexOf('({')).matchAll(/(\d+):"([0-9a-f]+)"/g)) files.push(m[2] + '.js');
fs.writeFileSync(out, files.join('\n') + '\n');
console.error(files.length, 'lazy chunks');
