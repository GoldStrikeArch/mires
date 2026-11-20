@file:Suppress("UnsafeCastFromDynamic")
private data class Scale(val low:Double,val high:Double,val dim:Double,val requested:Double,val vert:Boolean)
private fun scale(a:Double,b:Double,d:Double,v:Boolean)=Scale(a,b,rounded(d*2.0)/2.0,d,v)
private fun pixel(s:Scale,v:Double):Double { val pct=(v-s.low)/(s.high-s.low);return s.dim*(if(s.vert) 1.0-pct else pct) }
private fun valueAt(s:Scale,v:Double):Double { val q=if(s.vert)s.requested-v else v;return s.low+(s.high-s.low)*(q/s.requested) }
@JsName("Math") private external object NativeMath {fun round(x:Double):Double}
@JsName("devicePixelRatio") private external val pixelRatio:Double
@JsName("document") private external val doc:dynamic
@JsName("Path2D") private external class Path()
private fun newPath():dynamic=Path()
private fun rounded(x:Double)=NativeMath.round(x)
private fun half(i:Int)=i/2
private fun real(i:Int)=i.toDouble()
private fun num(x:Double):dynamic=x
private fun field(o:dynamic,k:String):dynamic=o[k]
private fun at(a:dynamic,i:Int):dynamic=a[i]
private fun empty(v:dynamic):Boolean=v==null
private fun numberAt(a:dynamic,i:Int):Double=a[i] as Double
private fun array():dynamic=emptyArray<Any?>()
private fun pair(a:dynamic,b:dynamic):dynamic=arrayOf<dynamic>(a,b)
private fun gapResult(span:Boolean,g:dynamic):dynamic=if(span)null else g
private fun appendValue(a:dynamic,v:dynamic){a.push(v)}
private fun size(a:dynamic):Int=a.length as Int
private fun line(p:dynamic,x:Double,y:Double){p.lineTo(x,y)}
private fun resize(canvas:dynamic,ctx:dynamic,w:Double,h:Double){canvas.width=rounded(w);canvas.height=rounded(h);ctx.strokeStyle="#172839"}
private fun draw(ctx:dynamic,stroke:dynamic,gaps:dynamic,w:Double,h:Double){
 ctx.save()
 try{
  val width = rounded(pixelRatio * 1000.0) / 1000.0
  val offset = (width % 2.0) / 2.0
  ctx.lineWidth = width
  ctx.lineJoin = "round"
  ctx.lineCap = "butt"
  ctx.translate(offset,offset)
  val bounds = newPath()
  bounds.rect(-width/2.0,-width/2.0,w+width,h+width)
  ctx.clip(bounds)
  if(size(gaps)>0){
   val mask=newPath();var end=0.0
   for(i in 0 until size(gaps)){val gap=at(gaps,i);val a=numberAt(gap,0);val b=numberAt(gap,1);if(b>a){if(a>end)mask.rect(end,0,a-end,h);end=b}}
   if(w>end)mask.rect(end,-5,w-end,h+10.0)
   ctx.clip(mask)
  }
  ctx.stroke(stroke)
 }finally{ctx.restore()}
}

private fun nearest(xs: dynamic, value: Double, lower: Int, upper: Int): Int {
  if (size(xs) == 0) return -1
    var lo: Int = lower
  var hi: Int = upper
  if (hi == 0) { hi = size(xs)-1 }
  while (hi > lo+1) {
    val mid: Int = half(lo+hi)
    if (numberAt(xs,mid) < value) { lo = mid } else { hi = mid }
  }
  if (value-numberAt(xs,lo) <= numberAt(xs,hi)-value) { return lo }
  return hi
}
private fun point(stroke: dynamic, points: dynamic, x: Double, y: Double): Unit {
  line(stroke,x,y)
  appendValue(points,pair(num(x),num(y)))
}
private fun finish(stroke: dynamic, points: dynamic, col: Double, y: Scale, low: Double, high: Double, first: Double, last: Double): Unit {
  val minY: Double = rounded(pixel(y,low))
  val maxY: Double = rounded(pixel(y,high))
  val firstY: Double = rounded(pixel(y,first))
  val lastY: Double = rounded(pixel(y,last))
  if (minY != maxY) {
    if (firstY != minY && lastY != minY) { point(stroke,points,col,minY) }
    if (firstY != maxY && lastY != maxY) { point(stroke,points,col,maxY) }
    point(stroke,points,col,lastY)
  }
}
private fun render(ctx: dynamic, x: Scale, y: Scale, xs: dynamic, ys: dynamic, span: Boolean): dynamic {
  val stroke: dynamic = newPath()
  val points: dynamic = array()
  val gaps: dynamic = array()
  val n: Int = size(ys)
  var first: Int = 0
  while (first<n && empty(at(ys,first))) { first+=1 }
  if (first<n) {
    var last: Int = n-1
    while (last>first && empty(at(ys,last))) { last-=1 }
    val dense: Boolean = real(last-first) >= x.dim*4.0
    var col: Double = rounded(pixel(x,numberAt(xs,first)))
    var limit: Double = valueAt(x,col+1.0)
    var has: Boolean = false
    var startY: Double = 0.0
    var endY: Double = 0.0
    var low: Double = 0.0
    var high: Double = 0.0
    var i: Int = first
    while (i<=last) {
      val xv: Double = numberAt(xs,i)
      if (dense && xv>=limit) {
        val next: Double = rounded(pixel(x,xv))
        if (next != col) {
          if (has) { finish(stroke,points,col,y,low,high,startY,endY) }
          has=false
          col=next
          limit=valueAt(x,col+1.0)
        }
      }
      if (!empty(at(ys,i))) {
        val v: Double = numberAt(ys,i)
        if (!dense) { point(stroke,points,rounded(pixel(x,xv)),rounded(pixel(y,v))) }
        else if (!has) {
          point(stroke,points,col,rounded(pixel(y,v)))
          has=true
          startY=v
          endY=v
          low=v
          high=v
        } else {
          endY=v
          if (v<low) { low=v }
          else if (v>high) { high=v }
        }
      }
      i+=1
    }
    if (has) { finish(stroke,points,col,y,low,high,startY,endY) }
    if (!span) {
      i=first
      while (i<=last) {
        if (empty(at(ys,i))) {
          val left: Double = rounded(pixel(x,numberAt(xs,i-1)))
          while (i<last && empty(at(ys,i))) { i+=1 }
          val right: Double = rounded(pixel(x,numberAt(xs,i)))
          if (right>=left) { appendValue(gaps,pair(num(left),num(right))) }
        }
        i+=1
      }
    }
  }
  draw(ctx,stroke,gaps,x.dim,y.dim)
  return pair(points,gapResult(span,gaps))
}
private fun precise(v: Double): dynamic {
  val n: Double = rounded(v*1000000.0)
  if (n == 0.0) { return num(0.0) }
  return num(n)
}
private fun frame(canvas: dynamic, ctx: dynamic, f: dynamic, span: Boolean): dynamic {
  val dims: dynamic = field(f,"size")
  val w: Double = numberAt(dims,0)
  val h: Double = numberAt(dims,1)
  resize(canvas,ctx,w,h)
  val xr: dynamic = field(f,"xRange")
  val yr: dynamic = field(f,"yRange")
  val x: Scale = scale(numberAt(xr,0),numberAt(xr,1),w,false)
  val y: Scale = scale(numberAt(yr,0),numberAt(yr,1),h,true)
  val data: dynamic = field(f,"data")
  val xs: dynamic = at(data,0)
  val paths: dynamic = array()
  var i: Int = 1
  while (i<size(data)) {
    appendValue(paths,render(ctx,x,y,xs,at(data,i),span))
    i+=1
  }
  var lo: Int = nearest(xs,x.low,0,size(xs)-1)
  var hi: Int = nearest(xs,x.high,0,size(xs)-1)
  if (hi>lo+1) {
    if (numberAt(xs,lo)<x.low) { lo+=1 }
    if (numberAt(xs,hi)>x.high) { hi-=1 }
  }
  val probes: dynamic = field(f,"probes")
  val answers: dynamic = array()
  i=0
  while (i<size(probes)) {
    val q: dynamic = at(probes,i)
    val xv: Double = numberAt(q,0)
    val px: Double = numberAt(q,2)
    val values: dynamic = pair(precise(pixel(x,xv)),precise(pixel(y,numberAt(q,1))))
    appendValue(values,precise(valueAt(x,px)))
    appendValue(values,precise(valueAt(y,numberAt(q,3))))
    appendValue(values,num(real(nearest(xs,xv,0,size(xs)-1))))
    appendValue(values,num(real(nearest(xs,valueAt(x,px),lo,hi))))
    appendValue(answers,values)
    i+=1
  }
  val result: dynamic = pair(field(canvas,"width"),field(canvas,"height"))
  appendValue(result,paths)
  appendValue(result,answers)
  return result
}
@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input:dynamic):dynamic{
 val canvas=doc.createElement("canvas");doc.body.appendChild(canvas)
 try{
  val ctx=canvas.getContext("2d");val result=array();val frames=input.frames
  for(i in 0 until size(frames))appendValue(result,frame(canvas,ctx,at(frames,i),input.spanGaps as Boolean))
  return result
 }finally{canvas.remove()}
}
