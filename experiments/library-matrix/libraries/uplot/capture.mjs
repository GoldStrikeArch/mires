// Observe the native canvas without replacing Path2D or intercepting the path builder.
function painted(run,input) {
  const contexts=new Set()
  const draws=[]
  const children=[...document.body.childNodes]
  const nativePath=globalThis.Path2D
  const canvasProto=HTMLCanvasElement.prototype
  const ctxProto=CanvasRenderingContext2D.prototype
  const getContext=canvasProto.getContext
  const stroke=ctxProto.stroke
  try {
    canvasProto.getContext=function(kind,...args) {
      const ctx=getContext.call(this,kind,...args)
      if(kind==='2d' && ctx) contexts.add(ctx)
      return ctx
    }
    ctxProto.stroke=function(...args) {
      const result=stroke.apply(this,args)
      if(!this.canvas.isConnected) throw Error('series painted on a detached canvas')
      if(args[0] instanceof nativePath) draws.push([this.canvas.width,this.canvas.height])
      return result
    }
    // uPlot schedules a later commit during setData. Sample the synchronous draw before its microtask.
    const result=run(input)
    if(contexts.size!==1) throw Error('expected one native 2D canvas, got '+contexts.size)
    const [ctx]=contexts
    if(ctx.canvas.isConnected || document.body.childNodes.length!==children.length ||
      children.some((node,i)=>node!==document.body.childNodes[i])) throw Error('chart leaked or replaced page DOM')
    const {width,height}=ctx.canvas
    const pixels=ctx.getImageData(0,0,width,height).data
    return {result,draws,width,height,pixels}
  } finally {
    canvasProto.getContext=getContext
    ctxProto.stroke=stroke
  }
}

function comparePixels(got,want,id) {
  if(got.width!==want.width || got.height!==want.height)throw Error('painted canvas size mismatch: '+id)
  const pos=got.pixels.findIndex((v,i)=>v!==want.pixels[i])
  if(pos!==-1){
    const p=pos>>>2
    throw Error('raster mismatch, '+id+' at ('+(p%got.width)+','+Math.floor(p/got.width)+') channel '+(pos%4)+
      ': '+got.pixels[pos]+' != '+want.pixels[pos])
  }
}

async function paintWitnesses(run,reference) {
  const xs=[0,15,30,45,60,75,90,105,120]
  const ys=[2,2,2,null,null,null,8,8,8]
  const frame=(values,size=[120,60])=>({size,xRange:[0,120],yRange:[0,10],data:[xs,values],probes:[]})
  const alphaNear=(chart,x,y)=>{
    let max=0
    for(let dy=-2;dy<=2;dy++)for(let dx=-2;dx<=2;dx++)
      max=Math.max(max,chart.pixels[4*((y+dy)*chart.width+x+dx)+3])
    return max
  }
  for(const spanGaps of [false,true]) {
    const input={spanGaps,frames:[frame(ys)]}
    const observed=await painted(run,input)
    const expected=await painted(reference,input)
    comparePixels(observed,expected,spanGaps?'spanning stroke':'clipped stroke')
    if(!observed.draws.length || !alphaNear(observed,16,48) || !alphaNear(observed,105,12))
      throw Error('missing native Path2D stroke on one side of the gap')
    if(Boolean(alphaNear(observed,60,30))!==spanGaps) throw Error('gap clipping did not change the painted pixels')
    const empty={...frame([],[95.5,55.5]),data:[[],[]],probes:[[2,3,40,12]]}
    const cleared=await painted(run,{spanGaps,frames:[frame(ys),empty]})
    const [[,,first],[w,h,paths,probes]]=cleared.result
    if (!first[0][0].length || w!==96 || h!==56 || cleared.width!==96 || cleared.height!==56)
      throw Error('populated frame or rounded bitmap size mismatch')
    if (cleared.pixels.some((value,i)=>i%4===3 && value!==0)) throw Error('empty frame retained painted pixels')
    if (JSON.stringify(paths)!==JSON.stringify([[[],spanGaps?null:[]]]) || probes[0][4]!==-1 || probes[0][5]!==-1)
      throw Error('empty frame retained vertices, gaps or a nearest index')
  }
  const input={spanGaps:true,frames:[frame(xs.map(()=>5),[84,38]),frame(xs.map(()=>8)),frame(xs.map(()=>2))]}
  const observed=await painted(run,input)
  comparePixels(observed,await painted(reference,input),'resize and replace')
  if(!observed.draws.some(([w,h])=>w===84&&h===38) || !observed.draws.some(([w,h])=>w===120&&h===60))
    throw Error('a resized frame was never stroked')
  if(alphaNear(observed,60,12)!==0 || !alphaNear(observed,60,48)) throw Error('old pixels survived data replacement')
}

export async function capture({moduleUrl,oracleUrl,cases,seeded}) {
  const {run}=await import(moduleUrl)
  const oracle=await import(oracleUrl)
  await paintWitnesses(run,oracle.run)
  const output={}
  for(const {id,input} of cases) {
    const got=await painted(run,input)
    const want=await painted(oracle.run,input)
    comparePixels(got,want,id)
    if(seeded && JSON.stringify(got.result)!==JSON.stringify(want.result))throw Error('path or probe mismatch: '+id)
    if(!seeded)output[id]=got.result
  }
  return output
}
