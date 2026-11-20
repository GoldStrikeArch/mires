import { writeFileSync } from 'node:fs'
const corpus = []
const response = (status,body,headers={}) => ({kind:'http',status,body,headers})
const ok = response(200,'{"answer":42}',{'Content-Type':'application/json'})
const defaults = {url:'https://matrix.invalid/items?a=hello%20world',method:'GET',headers:{},read:'json',limit:2,backoffLimit:1,maxRetryAfter:3,timeout:false,retryOnTimeout:false,throwHttpErrors:true,abort:'none',replies:[ok]}
function add(id,...cases){corpus.push({id,input:{cases:cases.map(c=>({...defaults,...c}))}})}
add('get-json',{})
add('text',{read:'text',replies:[response(200,'雪')]})
add('request-json',{method:'POST',json:{name:'A & B',ids:[0,1],flag:false}},{method:'PUT',headers:{Accept:'custom/format','Content-Type':'application/vnd.example+json'},json:[1,2]})
add('construction-errors-continue',
  {json:{invalid:'GET body'}},{method:'TRACE'},{headers:{'bad header':'value'}},{})
add('empty-content-type',
  {method:'POST',headers:{'cOnTeNt-TyPe':''},json:[]},
  {method:'POST',json:[]},{method:'POST',headers:{'content-type':'text/plain'},json:[]})
const retryOnce=[{kind:'network'},ok]
add('method-case',
  {method:'custom',methods:['CUSTOM'],replies:retryOnce},
  {method:'custom',methods:['custom'],replies:retryOnce},
  {method:'CUSTOM',methods:['custom'],replies:retryOnce},
  {method:'pAtCh',methods:['PaTcH'],replies:retryOnce},
  {method:'qUeRy',replies:retryOnce})
add('empty-and-malformed',{replies:[response(204,null)]},{replies:[response(200,'{bad')]},{read:'text',replies:[response(204,null)]})
add('http-errors',{replies:[response(404,'{"code":"missing"}',{'Content-Type':'application/problem+json'})]},{replies:[response(400,'plain error')]},{replies:[response(404,'{bad',{'Content-Type':'application/json'})]},{replies:[response(404,'')]})
add('throw-disabled',{throwHttpErrors:false,replies:[response(503,'{"offline":true}')]})
add('retry-twice',{replies:[response(500,'x'),response(502,'y'),ok]})
add('retry-limit',{limit:1,replies:[response(503,'busy')]},{limit:0,replies:[{kind:'network'}]})
add('retry-methods',{method:'POST',json:{a:1},replies:[response(503,'busy'),ok]},{method:'POST',json:{a:1},methods:['post'],replies:[response(503,'busy'),ok]},{method:'GET',methods:[],replies:[{kind:'network'},ok]})
add('retry-statuses',{statusCodes:[409],replies:[response(409,'conflict'),ok]},{statusCodes:[],replies:[response(503,'busy'),ok]})
add('retry-after',{replies:[response(429,'',{'Retry-After':'60'}),ok]},{replies:[response(503,'',{'Retry-After':'invalid'}),ok]},{replies:[response(413,'too large')]},{replies:[response(413,'',{'Retry-After':'0.001'}),ok]})
add('network',{replies:[{kind:'network'},ok]},{replies:[{kind:'network'}]})
add('abort',{abort:'before'},{abort:'fetch'},{abort:'retry',replies:[response(503,'busy'),ok]})
// Allow the native body reader to record the dispatch before timing out the stalled reply.
add('timeout',{timeout:150,replies:[{kind:'stall'}]},{timeout:150,retryOnTimeout:true,replies:[{kind:'stall'},ok]})
add('bench-requests',...Array.from({length:24},(_,i)=>({url:'https://matrix.invalid/record/'+i,method:i%2?'POST':'GET',...(i%2?{json:{i,value:'sample'}}:{})})))
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'ky',cases:corpus})+'\n')
