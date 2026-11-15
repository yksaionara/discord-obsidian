// usage: node obj.js <file> <anchorSubstring> [--json]
// Finds the smallest object literal containing anchor, prints key => rendered value
const fs=require('fs'),acorn=require('acorn');
const [file,anchor]=process.argv.slice(2);const s=fs.readFileSync(file,'utf8');
let at=s.indexOf(anchor);if(at<0){console.error('anchor not found');process.exit(1)}
function enclosingObjectStart(pos){// scan back for '{' with depth, naive about strings but ok to try multiple
  let depth=0;for(let i=pos;i>=0;i--){const c=s[i];if(c==='}')depth++;else if(c==='{'){if(depth===0)return i;depth--}}return -1}
let start=enclosingObjectStart(at),node=null;
for(let tries=0;tries<50&&start>=0;tries++){try{node=acorn.parseExpressionAt(s,start,{ecmaVersion:'latest'});if(node.type==='ObjectExpression'&&node.end>at)break;node=null}catch(e){node=null}start=enclosingObjectStart(start-1)}
if(!node){console.error('no object');process.exit(1)}
const src=n=>s.slice(n.start,n.end);
function render(v,params){
  if(!v)return '?';
  switch(v.type){
  case 'Literal':return JSON.stringify(v.value);
  case 'TemplateLiteral':{let o='';v.quasis.forEach((q,i)=>{o+=q.value.cooked;if(v.expressions[i])o+='{'+(v.expressions[i].type==='Identifier'?(params[v.expressions[i].name]||v.expressions[i].name):src(v.expressions[i]))+'}'});return o}
  case 'ArrowFunctionExpression':case 'FunctionExpression':{const p={};v.params.forEach((x,i)=>{if(x.type==='Identifier')p[x.name]='arg'+i});
    let body=v.body; if(body.type==='BlockStatement'){const r=body.body.find(b=>b.type==='ReturnStatement');if(r)body=r.argument;else return 'fn'}
    return render(body,p)}
  case 'BinaryExpression':if(v.operator==='+')return render(v.left,params)+render(v.right,params);return src(v);
  case 'Identifier':return '{'+(params[v.name]||v.name)+'}';
  default:return src(v).slice(0,200)}
}
const out={};for(const p of node.properties){if(p.type!=='Property')continue;const k=p.key.type==='Identifier'?p.key.name:(p.key.value??src(p.key));out[k]=render(p.value,{})}
if(process.argv.includes('--json'))console.log(JSON.stringify(out,null,1));else for(const k in out)console.log(k+'\t'+out[k]);
console.error('props:',Object.keys(out).length,'at',node.start);
