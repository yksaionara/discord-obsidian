const fs=require('fs');const dir=process.argv[2];const pats=process.argv.slice(3);
for(const f of fs.readdirSync(dir)){const s=fs.readFileSync(dir+'/'+f,'utf8');for(const p of pats){let i=s.indexOf(p);let n=0;while(i>=0&&n<3){console.log(p,'|',f,i,'|',s.slice(Math.max(0,i-120),i+160).replace(/\n/g,' '));n++;i=s.indexOf(p,i+1)}}}
