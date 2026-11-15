// split webpack chunks into one beautified file per module: out/<moduleId>.js ; index.tsv: id chunk bytes
const fs=require('fs'),acorn=require('acorn'),beautify=require('js-beautify').js;
const [dir,out]=process.argv.slice(2);fs.mkdirSync(out,{recursive:true});
const idx=fs.createWriteStream(out+'/../modules.tsv');let n=0,skipped=0;
function modulesObjects(ast){const res=[];
 (function walk(node,depth){if(!node||typeof node.type!=='string'||depth>12)return;
  if(node.type==='ObjectExpression'&&node.properties.length&&node.properties.every(p=>p.type==='Property'&&(p.key.type==='Literal'&&typeof p.key.value==='number'||(p.key.type==='Identifier'&&/^\d+$/.test(p.key.name)))&&(p.method||/Function/.test(p.value.type)))){res.push(node);return}
  for(const k in node){const v=node[k];if(Array.isArray(v))v.forEach(x=>walk(x,depth+1));else if(v&&typeof v.type==='string')walk(v,depth+1)}})(ast,0);return res}
for(const f of fs.readdirSync(dir)){const s=fs.readFileSync(dir+'/'+f,'utf8');let ast;
 try{ast=acorn.parse(s,{ecmaVersion:'latest',sourceType:'script',allowHashBang:true})}catch(e){console.error('parse fail',f,e.message);continue}
 for(const obj of modulesObjects(ast))for(const p of obj.properties){const id=p.key.value??p.key.name;const body=s.slice(p.start,p.end);
  if(/^\d+\(\w+\)\{\w+\.exports=JSON\.parse\('/.test(body)&&!/"_hash"|en-US/.test(f)){skipped++;continue}
  let code=body;if(body.length<3e6)try{code=beautify(body,{indent_size:2,max_preserve_newlines:1})}catch(e){}
  fs.writeFileSync(`${out}/${id}.js`,`// module ${id} from ${f}\n`+code);idx.write(`${id}\t${f}\t${body.length}\n`);n++}}
idx.end();console.log('modules',n,'skipped json',skipped);
