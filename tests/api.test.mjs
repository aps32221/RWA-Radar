import {test} from 'node:test';
import assert from 'node:assert/strict';
const base=process.env.API_URL||'http://127.0.0.1:8080/api';
const post=(path,body)=>fetch(base+path,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});
test('API health, normalization, quote, simulation, persistence and fail-closed execution',async()=>{
 const health=await (await fetch(base+'/health')).json();assert.equal(health.mode,'demo');assert.equal(health.broadcastEnabled,false);
 const assets=await (await fetch(base+'/assets')).json();assert.equal(assets.length,8);assert.equal(assets[0].providers.length,3);
 const nvda=await (await fetch(base+'/assets/NVDA')).json();assert.equal(nvda.symbol,'NVDA');assert.ok(nvda.score>=0&&nvda.score<=100);
 const request={asset:'NVDA',provider:'xStocks',side:'sell',amountUSD:.01};
 const quote=await (await post('/trades/quote',request)).json();assert.ok(quote.expectedOutput>0);assert.equal(quote.mode,'demo');
 assert.equal((await post('/trades/quote',{...request,amountUSD:-1})).status,400);
 assert.equal((await post('/trades/quote',{...request,provider:'unknown'})).status,400);
 const blocked=await (await post('/trades/simulate',{...request,amountUSD:6})).json();assert.equal(blocked.success,false);
 const simulation=await (await post('/trades/simulate',request)).json();assert.equal(simulation.success,true);assert.ok(simulation.id);
 const history=await (await fetch(base+'/history')).json();assert.ok(history.some(s=>s.id===simulation.id));
 assert.equal((await post('/trades/execute',{simulationId:simulation.id})).status,501);
 const telemetry=await (await fetch(base+'/telemetry')).json();assert.ok(telemetry.some(t=>t.endpoint==='/api/trades/simulate'&&t.requests>=2));
});
