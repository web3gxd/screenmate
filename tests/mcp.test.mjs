import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const bin = process.env.SCREENMATE_MCP_BIN || fileURLToPath(new URL('../mcp_server/target/debug/screenmate-mcp.exe', import.meta.url));
const frame = changed => ({ image_base64:'/9j/2Q==',mime_type:'image/jpeg',timestamp:'2026-01-01T00:00:00Z',width:4,height:2,changed });
test('full stdio and authenticated local RPC exchange', async () => {
  const root=await mkdtemp(join(tmpdir(),'screenmate-e2e-'));
  let token='c'.repeat(64), captures=0;
  const server=createServer((req,res)=>{
    assert.equal(req.headers.authorization,`Bearer ${token}`); assert.equal(req.url,'/rpc');
    let raw=''; req.on('data',c=>{raw+=c}); req.on('end',()=>{
      const call=JSON.parse(raw);
      if(call.method==='look') {
        captures++;
        const result=frame(captures!==2);
        if(captures===1) {
          // Like tiny_http for a large JPEG, omit Content-Length and stream
          // multiple chunks. Exercise the actual stdio -> HTTP -> MCP path.
          result.image_base64='/9j/'.repeat(16384);
          const body=JSON.stringify({ok:true,result});
          res.writeHead(200,{'Transfer-Encoding':'chunked'});
          for(let i=0;i<body.length;i+=8192)res.write(body.slice(i,i+8192));
          res.end();
        } else res.end(JSON.stringify({ok:true,result}));
      }
      else {res.statusCode=400;res.end(JSON.stringify({ok:false,error:'пауза'}));}
    });
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const session=join(root,'Screenmate','session.json');
  await mkdir(join(root,'Screenmate'));
  const save=()=>writeFile(session,JSON.stringify({port:server.address().port,token}));
  await save();
  const child=spawn(bin,[],{stdio:['pipe','pipe','pipe'],env:{...process.env,LOCALAPPDATA:root}});
  const lines=createInterface({input:child.stdout});
  let nextId=0;
  const call=(method,params={})=>new Promise((resolve,reject)=>{
    lines.once('line',line=>{try{resolve(JSON.parse(line))}catch(e){reject(e)}});
    child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:++nextId,method,params})+'\n');
    child.once('error',reject);
  });
  try {
    const init=await call('initialize',{protocolVersion:'2025-06-18',capabilities:{},clientInfo:{name:'test',version:'1'}});
    assert.equal(init.result.protocolVersion,'2025-06-18');
    assert.equal((await call('tools/list')).result.tools.length,6);
    const look=await call('tools/call',{name:'look',arguments:{monitor:'123'}});
    assert.equal(look.result.content[1].type,'image');
    assert.equal(look.result.content[1].data.length,65536);
    token='d'.repeat(64); await save();
    const diff=await call('tools/call',{name:'look_diff',arguments:{}});
    assert.equal(diff.result.content[0].text,'без изменений');
    const watched=await call('tools/call',{name:'watch',arguments:{seconds:1,interval:0.2}});
    assert.equal(captures,7); assert.equal(watched.result.content.at(-1).type,'image');
    const err=await call('tools/call',{name:'say',arguments:{text:'hello'}});
    assert.equal(err.result.isError,true); assert.match(err.result.content[0].text,/пауза/);
    const bad=await call('tools/call',{name:'watch',arguments:{seconds:1,interval:0}});
    assert.equal(bad.result.isError,true);
  } finally {child.kill();lines.close();await new Promise(resolve=>server.close(resolve));await rm(root,{recursive:true,force:true});}
});
