package uplot
import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel
object Main {
 type V=js.Dynamic
 case class Scale(low:Double,high:Double,dim:Double,requested:Double,vert:Boolean)
 def scale(a:Double,b:Double,d:Double,v:Boolean)=Scale(a,b,rounded(d*2.0)/2.0,d,v)
 def pixel(s:Scale,v:Double):Double={val pct=(v-s.low)/(s.high-s.low);s.dim*(if(s.vert) 1.0-pct else pct)}
 def valueAt(s:Scale,v:Double):Double={val q=if(s.vert)s.requested-v else v;s.low+(s.high-s.low)*(q/s.requested)}
 def rounded(x:Double):Double=js.Dynamic.global.Math.round(x).asInstanceOf[Double]
 def half(i:Int)=i/2
 def real(i:Int)=i.toDouble
 def field(o:V,k:String):V=o.selectDynamic(k)
 def at(a:V,i:Int):V=a.asInstanceOf[js.Array[V]](i)
 def numberAt(a:V,i:Int):Double=a.asInstanceOf[js.Array[Double]](i)
 def num(v:Double):V=v.asInstanceOf[V]
 def empty(v:V):Boolean=v==null
 def size(v:V):Int=v.length.asInstanceOf[Int]
 def array():V=js.Array[V]().asInstanceOf[V]
 def pair(a:V,b:V):V=js.Array(a,b).asInstanceOf[V]
 def gapResult(span:Boolean,a:V):V=if(span)null.asInstanceOf[V] else a
 def appendValue(xs:V,v:V):Unit={xs.push(v);()}
 def newPath():V=js.Dynamic.newInstance(js.Dynamic.global.Path2D)()
 def line(p:V,x:Double,y:Double):Unit={p.lineTo(x,y);()}
 def resize(canvas:V,ctx:V,w:Double,h:Double):Unit={canvas.width=rounded(w);canvas.height=rounded(h);ctx.strokeStyle="#172839"}
 def draw(ctx:V,path:V,gaps:V,w:Double,h:Double):Unit={
  ctx.save()
  try{
   val width = rounded(js.Dynamic.global.devicePixelRatio.asInstanceOf[Double] * 1000.0) / 1000.0
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
    for(i<-0 until size(gaps)){val gap=at(gaps,i);val left=numberAt(gap,0);val right=numberAt(gap,1)
     if(right>left){if(left>end)mask.rect(end,0,left-end,h);end=right}}
    if(w>end)mask.rect(end,-5,w-end,h+10)
    ctx.clip(mask)
   };ctx.stroke(path)
  }finally{ctx.restore()}
 }

 def nearest(xs: V, value: Double, lower: Int, upper: Int): Int = {
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
 def point(stroke: V, points: V, x: Double, y: Double): Unit = {
   line(stroke,x,y)
   appendValue(points,pair(num(x),num(y)))
 }
 def finish(stroke: V, points: V, col: Double, y: Scale, low: Double, high: Double, first: Double, last: Double): Unit = {
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
 def render(ctx: V, x: Scale, y: Scale, xs: V, ys: V, span: Boolean): V = {
   val stroke: V = newPath()
   val points: V = array()
   val gaps: V = array()
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
 def precise(v: Double): V = {
   val n: Double = rounded(v*1000000.0)
   if (n == 0.0) { return num(0.0) }
   return num(n)
 }
 def frame(canvas: V, ctx: V, f: V, span: Boolean): V = {
   val dims: V = field(f,"size")
   val w: Double = numberAt(dims,0)
   val h: Double = numberAt(dims,1)
   resize(canvas,ctx,w,h)
   val xr: V = field(f,"xRange")
   val yr: V = field(f,"yRange")
   val x: Scale = scale(numberAt(xr,0),numberAt(xr,1),w,false)
   val y: Scale = scale(numberAt(yr,0),numberAt(yr,1),h,true)
   val data: V = field(f,"data")
   val xs: V = at(data,0)
   val paths: V = array()
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
   val probes: V = field(f,"probes")
   val answers: V = array()
   i=0
   while (i<size(probes)) {
     val q: V = at(probes,i)
     val xv: Double = numberAt(q,0)
     val px: Double = numberAt(q,2)
     val values: V = pair(precise(pixel(x,xv)),precise(pixel(y,numberAt(q,1))))
     appendValue(values,precise(valueAt(x,px)))
     appendValue(values,precise(valueAt(y,numberAt(q,3))))
     appendValue(values,num(real(nearest(xs,xv,0,size(xs)-1))))
     appendValue(values,num(real(nearest(xs,valueAt(x,px),lo,hi))))
     appendValue(answers,values)
     i+=1
   }
   val result: V = pair(field(canvas,"width"),field(canvas,"height"))
   appendValue(result,paths)
   appendValue(result,answers)
   return result
 }
 @JSExportTopLevel("run")
 def run(input:V):V={
  val doc=js.Dynamic.global.document;val canvas=doc.createElement("canvas");doc.body.appendChild(canvas)
  try{
   val ctx=canvas.getContext("2d");val frames=input.frames;val out=array()
   for(i<-0 until size(frames)){appendValue(out,frame(canvas,ctx,at(frames,i),input.spanGaps.asInstanceOf[Boolean]))}
   out
  }finally{canvas.remove()}
 }
}
