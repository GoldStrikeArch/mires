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

// Pre-merge review (2026-11-20). A retry that waits on a timing header traces no 'backoff'; a
// header Ky cannot read falls back to the exponential backoff. `maxRetryAfter: 3` caps every wait.
const timed = (status,headers) => ({replies:[response(status,'',headers),ok]})
add('retry-timing-headers',
  timed(429,{'RateLimit-Reset':'2'}),timed(503,{'X-RateLimit-Retry-After':'1'}),
  timed(429,{'X-RateLimit-Reset':'1'}),timed(429,{'X-Rate-Limit-Reset':'1'}),
  timed(429,{'Retry-After':'invalid','RateLimit-Reset':'1'}),timed(429,{'RateLimit-Reset':'soon'}),
  timed(429,{'X-RateLimit-Reset':'later','X-Rate-Limit-Reset':'1'}),
  timed(413,{'RateLimit-Reset':'1'}),timed(500,{'Retry-After':'1'}),
  {statusCodes:[500],replies:[response(413,'',{'Retry-After':'1'}),ok]})
add('retry-after-values',
  timed(413,{'Retry-After':''}),timed(429,{'Retry-After':''}),
  timed(429,{'Retry-After':'\u00a00'}),timed(429,{'Retry-After':'\u000b0'}),timed(429,{'Retry-After':'\f0'}),
  timed(429,{'Retry-After':' 0'}),timed(429,{'Retry-After':'0000'}),timed(429,{'Retry-After':'²'}),
  timed(429,{'Retry-After':'1e3'}),timed(429,{'Retry-After':'-1'}),timed(429,{'Retry-After':'+1'}),
  timed(429,{'Retry-After':'99999999999999999999999'}),timed(429,{'Retry-After':'9'.repeat(400)}))
add('retry-after-dates',
  timed(429,{'Retry-After':'Thu, 01 Jan 1970 00:00:00 GMT'}),timed(429,{'Retry-After':'Fri, 01 Jan 2100 00:00:00 GMT'}),
  timed(429,{'Retry-After':'Mon, 31 Feb 2000 00:00:00 GMT'}),timed(429,{'Retry-After':'Sat, 31 Dec 2016 23:59:60 GMT'}),
  timed(429,{'Retry-After':'Sat, 31 Dec 2016 23:59:61 GMT'}),timed(429,{'Retry-After':'Sat, 31 Dec 2016 24:00:00 GMT'}),
  timed(429,{'Retry-After':'Sat, 31 Dec 2016 23:60:00 GMT'}),timed(429,{'Retry-After':'Thu, 01 Jan 1970 00:00:00 gmt'}),
  timed(429,{'Retry-After':'Mon, 01 Jan 1970 00:00:00 GMT'}),timed(429,{'Retry-After':'Thu, 1 Jan 1970 00:00:00 GMT'}),
  timed(429,{'Retry-After':'Tue, 29 Feb 0000 00:00:00 GMT'}),timed(429,{'Retry-After':'Tue, 29 Feb 0004 00:00:00 GMT'}),
  timed(429,{'Retry-After':'Thu, 29 Feb 1900 00:00:00 GMT'}),timed(429,{'Retry-After':'Tue, 29 Feb 2000 00:00:00 GMT'}),
  timed(429,{'Retry-After':'Thu, 01 Jan 1970 00:00:00 GMT '}),timed(429,{'Retry-After':'Thu, 01 Jen 1970 00:00:00 GMT'}),
  timed(429,{'Retry-After':'Sunday, 06-Nov-94 08:49:37 GMT'}),timed(429,{'Retry-After':'Tuesday, 29-Feb-00 00:00:00 GMT'}),
  timed(429,{'Retry-After':'Sunday, 31-Feb-94 08:49:37 GMT'}),timed(429,{'Retry-After':'Sun, 06-Nov-94 08:49:37 GMT'}),
  timed(429,{'Retry-After':'Sun Nov  6 08:49:37 1994'}),timed(429,{'Retry-After':'Sun Nov 06 08:49:37 1994'}),
  timed(429,{'Retry-After':'Sun Nov 31 08:49:37 1994'}),timed(429,{'Retry-After':'Sun Nov 6 08:49:37 1994'}),
  timed(503,{'X-RateLimit-Reset':'Sun Nov  6 08:49:37 1994'}))
const latin1 = contentType => ({replies:[response(400,'é',{'Content-Type':contentType})]})
add('error-data-charset',
  latin1('text/plain; charset=iso-8859-1'),{replies:[response(400,'ab',{'Content-Type':'text/plain; charset=utf-16le'})]},
  {replies:[response(400,'ab',{'Content-Type':'text/plain; charset=utf-16be'})]},
  {replies:[response(400,'€',{'Content-Type':'text/plain; charset=windows-1252'})]},
  latin1('text/plain; charset="iso-8859-1"'),latin1('text/plain; CHARSET=ISO-8859-1'),latin1('text/plain; charset = latin1'),
  latin1('text/plain;\u00a0charset=latin1'),latin1('text/plain; charset=latin1\u00a0'),latin1('text/plain; charset=latin1\u0085'),
  latin1('text/plain; charset=latin1\u000b'),latin1('text/plain; charset=bogus'),latin1('text/plain; charset='),
  latin1('text/plain; charset="latin1'),latin1('text/plain; charset=""'),latin1('text/plain; xcharset=latin1'),
  latin1('text/plain; foo=1; charset=latin1'),latin1('text/plain; charset=utf-8; charset=latin1'),
  latin1('text/plain; charset=latin1, x'),latin1('text/plain;charset=latin1;x'),latin1('charset=latin1'),
  {replies:[response(400,'"é"',{'Content-Type':'application/json; charset=iso-8859-1'})]},
  {replies:[response(400,null,{'Content-Type':'text/plain; charset=latin1'})]})
const typed = contentType => ({replies:[response(404,'{"a":1}',{'Content-Type':contentType})]})
add('json-content-type',
  typed('application/json\u00a0'),typed('application/json\u0085'),typed('application/json\u001f'),
  typed('application/json\u000b'),typed('application/json\f'),typed('\u00a0application/json'),
  typed('json'),typed('x+json'),typed('/json'),typed('+json'),typed('a/+json'),typed('text/plain, application/json'),
  typed('application/vnd.api+json'),typed('application/x-json'),typed('application/x.json'),typed('APPLICATION/JSON'),
  typed('application/jsonp'),typed('application/json ;x'),typed('application/problem+json; charset=utf-8'),
  {replies:[response(404,' ',{'Content-Type':'application/json'})]},{replies:[response(404,'null',{'Content-Type':'application/json'})]},
  {replies:[response(404,null,{'Content-Type':'application/json'})]},{replies:[response(404,'0')]})
add('options-validation',
  {limit:-1},{limit:1.5},{limit:1.5,replies:[{kind:'network'},{kind:'network'},ok]},{limit:'2'},{limit:null},{limit:true},{backoffLimit:-1},{backoffLimit:'1'},
  {maxRetryAfter:-1},{maxRetryAfter:null},{timeout:-5},{timeout:2**31},{timeout:2147483647.5},{timeout:true},
  {timeout:'100'},{timeout:null},{retryOnTimeout:'yes'},{retryOnTimeout:null},{throwHttpErrors:'yes'},
  {throwHttpErrors:null},{throwHttpErrors:0},{methods:[1],replies:retryOnce},{methods:['get',null]},{methods:'get'},
  {methods:null},{methods:{}},{statusCodes:['x']},{statusCodes:[500.5]},{statusCodes:500},{statusCodes:[null]},
  {timeout:2147483647},{limit:0,backoffLimit:0,maxRetryAfter:0})
add('options-error-order',
  {limit:-1,methods:'x'},{methods:'x',statusCodes:['y']},{methods:[1],statusCodes:'x'},{retryOnTimeout:'yes',methods:'x'},
  {throwHttpErrors:'yes',methods:'x'},{timeout:2**31,throwHttpErrors:'yes'},{timeout:2**31,limit:-1},
  {timeout:2**31,method:'TRACE'},{timeout:2**31,json:{a:1}},{headers:{'bad header':'v'},methods:'x'},
  {headers:{'bad header':'v'},timeout:2**31},{abort:'before',timeout:2**31},{abort:'before',methods:'x'},
  {abort:'before',method:'TRACE'})
add('options-omitted',
  {maxRetryAfter:undefined,replies:[response(429,'',{'Retry-After':'0'}),ok]},
  {maxRetryAfter:undefined,replies:[response(429,'',{'RateLimit-Reset':'1000000000'}),ok]},
  {maxRetryAfter:undefined,replies:[response(429,'',{'Retry-After':'Thu, 01 Jan 1970 00:00:00 GMT'}),ok]},
  {backoffLimit:undefined,limit:1,replies:retryOnce},
  {limit:undefined,replies:[{kind:'network'},{kind:'network'},{kind:'network'}]},
  {timeout:undefined},{throwHttpErrors:undefined,replies:[response(404,'gone')]},
  {retryOnTimeout:undefined,timeout:150,replies:[{kind:'stall'},ok]})
add('request-headers',
  {headers:{'X-A':null}},{headers:{Accept:null}},{headers:{Accept:5}},{headers:{Accept:''}},{headers:{Accept:' '}},
  {headers:{Accept:'a',accept:'b'}},{headers:{accept:'b',Accept:'a'}},
  {method:'POST',headers:{'Content-Type':'x','content-type':'y'},json:[]},{method:'POST',json:null},
  {headers:{'\u212aey':'kelvin'}},{headers:{'\u0130d':'dotted'}},{headers:{'X-A':'\u0100'}},{headers:{'X-A':'a\nb'}})
add('methods-and-urls',
  {method:'bad method'},{method:'CONNECT'},{method:'track'},{method:'Get'},{url:'not a url'},
  {url:'https://matrix.invalid/a b/ü'},{method:'post',methods:['POST'],replies:retryOnce},
  {method:'options',replies:retryOnce},{method:'PoSt',methods:['pOsT'],replies:retryOnce},
  {method:'delete',json:{a:1},replies:retryOnce},{method:'head',read:'text',replies:[response(200,'')]})
add('reply-errors',
  {replies:[response(600,null)]},{replies:[response(199,null)]},{replies:[response(204,'x')]},
  {replies:[response(200,'{}',{'bad header':'x'})]},{limit:0,replies:[response(600,null)]},
  {throwHttpErrors:false,replies:[response(503,'x')]},{throwHttpErrors:false,read:'text',replies:[response(503,'x')]})
// Past int32 and past 2^53: retry limits, status codes and backoff delays are doubles in Ky.
add('large-numbers',
  {limit:4294967296,replies:retryOnce},{limit:9007199254740992,replies:retryOnce},{limit:1e300,replies:retryOnce},
  {statusCodes:[4294967796],replies:[response(500,'x'),ok]},{statusCodes:[500,1e300],replies:[response(500,'x'),ok]},
  {limit:200,backoffLimit:0,replies:[...Array(200).fill({kind:'network'}),ok]})
// Deep and wide values pass through the request body, the response body and the trace.
const nested = depth => { let value = 0; for (let i = 0; i < depth; i++) value = [value]; return value }
add('deep-and-wide',
  {method:'POST',json:nested(1000)},{replies:[response(200,'['.repeat(1000)+']'.repeat(1000))]},
  {read:'text',replies:[response(200,'x'.repeat(200000))]},
  {replies:[response(404,'['.repeat(1000)+']'.repeat(1000),{'Content-Type':'application/json'})]})
add('ok-bodies',
  {replies:[response(200,'"x"',{'Content-Type':'application/json; charset=utf-16le'})]},
  {read:'text',replies:[response(200,'é',{'Content-Type':'text/plain; charset=latin1'})]},
  {read:'text',replies:[response(200,'\ufeffbom')]},{replies:[response(200,' {"a" : [1, 2.5e3] } ')]})
add('abort-more',{abort:'fetch',limit:0},{abort:'retry',limit:1,replies:retryOnce},{abort:'before',limit:0})
add('bench-requests',...Array.from({length:24},(_,i)=>({url:'https://matrix.invalid/record/'+i,method:i%2?'POST':'GET',...(i%2?{json:{i,value:'sample'}}:{})})))
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'ky',cases:corpus})+'\n')
