package matrixdiff
import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel
import scala.collection.mutable.ArrayBuffer
import java.nio.charset.StandardCharsets

object Main {
  private final case class Run(var kind: Int, var count: Int, var prev: Int)
  private final case class Path(var old: Int, var last: Int)
  private final case class Change(var kind: Int, var count: Int, var value: String)
  private final case class Hunk(var oldStart: Int, var oldLines: Int, var newStart: Int, var newLines: Int, var lines: ArrayBuffer[String])
  private final case class Patch(var meta: js.Any, var hunks: ArrayBuffer[Hunk])

  private def smaller(a:Int,b:Int)=math.min(a,b)
  private def bigger(a:Int,b:Int)=math.max(a,b)
  private def divide(a:Int,b:Int)=a/b
  private def flip[T](a:ArrayBuffer[T]):Unit = {var i=0;while(i<a.length/2){val j=a.length-i-1;val t=a(i);a(i)=a(j);a(j)=t;i+=1}}
  private def pop[T](a:ArrayBuffer[T]):Unit={a.remove(a.length-1);()}
  private def utf8(s:String):ArrayBuffer[Int]=ArrayBuffer.from(s.getBytes(StandardCharsets.UTF_8).map(_ & 255))
  private def decodeUtf8(b:ArrayBuffer[Int])=new String(b.iterator.map(_.toByte).toArray,StandardCharsets.UTF_8)
  private def cut(s:String,a:Int,b:Int)=s.substring(a,b)
  private def code(s:String,i:Int)=s.charAt(i).toInt
  private def unit(c:Int)=c.toChar.toString
  private def starts(s:String,p:String)=s.startsWith(p)
  private def ends(s:String,p:String)=s.endsWith(p)
  private def contains(s:String,p:String)=s.contains(p)
  private def replaceAll(s:String,a:String,b:String)=s.replace(a,b)
  private def lower(s:String)=s.toLowerCase
  private def indexOf(s:String,p:String)=s.indexOf(p)
  private def lastIndex(s:String,p:String)=s.lastIndexOf(p)
  private def split(s:String,sep:String)=ArrayBuffer.from(s.asInstanceOf[js.Dynamic].split(sep).asInstanceOf[js.Array[String]])
  private def strAt(s:String,i:Int)=split(s,"|")(i)
  private def join(a:ArrayBuffer[String],sep:String)=a.mkString(sep)
  private def dec(n:Int)=n.toString
  private def parseInt(s:String)=s.toInt
  private def compact(v:js.Any)=js.JSON.stringify(v)
  private def pretty(v:js.Any)=js.Dynamic.global.JSON.stringify(v,null,2).asInstanceOf[String]
  private def dictionary():js.Any=js.Dynamic.global.Object.create(null)
  private def obj():js.Any=js.Dynamic.literal()
  private def arr(n:Int):js.Any=new js.Array[js.Any](n)
  private def defined(v:js.Any):Boolean=v!=null && !js.isUndefined(v)
  private def get(v:js.Any,k:String):js.Any=if(defined(v)) v.asInstanceOf[js.Dynamic].selectDynamic(k) else js.undefined
  private def put(v:js.Any,k:String,x:js.Any):Unit=v.asInstanceOf[js.Dynamic].updateDynamic(k)(x)
  private def assign(v:js.Any,i:Int,x:js.Any):Unit=v.asInstanceOf[js.Array[js.Any]](i)=x
  private def item(v:js.Any,i:Int):js.Any=v.asInstanceOf[js.Array[js.Any]](i)
  private def names(v:js.Any)=ArrayBuffer.from(js.Object.keys(v.asInstanceOf[js.Object]))
  private def sortNames(a:ArrayBuffer[String]):Unit={a.sortInPlace();()}
  private def size(v:js.Any)=v.asInstanceOf[js.Array[js.Any]].length
  private def flag(v:js.Any,k:String)=defined(get(v,k)) && get(v,k).asInstanceOf[Boolean]
  private def undefinedValue():js.Any=js.undefined
  private def text(v:js.Any)=v.asInstanceOf[String]
  private def string(s:String):js.Any=s
  private def integer(v:js.Any)=v.asInstanceOf[Int]
  private def num(n:Int):js.Any=n
  private def boolVal(b:Boolean):js.Any=b
  private def isArray(v:js.Any)=js.Array.isArray(v)
  private def isString(v:js.Any)=js.typeOf(v)=="string"
  private def isObject(v:js.Any)=defined(v) && js.typeOf(v)=="object"
  private def ensure(b:Boolean):Unit=require(b,"invalid patch")

// The edit history is an arena of linked runs, shared between neighbouring diagonals.
  private def record(arena: ArrayBuffer[Run], last: Int, kind: Int, count: Int, separate: Boolean): Int = {
      if (count == 0) {
          return last
      }
      var prev: Int = last
      var n: Int = count
      if (separate) {
          for (q <- 0 until count) {
              arena.append(Run(kind, 1, prev))
              prev = arena.length - 1
          }
          return prev
      }
      if (prev != 0 && arena(prev).kind == kind) {
          n += arena(prev).count
          prev = arena(prev).prev
      }
      arena.append(Run(kind, n, prev))
      return arena.length - 1
  }
  private def advance(p: Path, k: Int, a: ArrayBuffer[String], b: ArrayBuffer[String], arena: ArrayBuffer[Run], separate: Boolean): Int = {
      val initial: Int = p.old
      var j: Int = initial - k
      while (p.old + 1 < a.length && j + 1 < b.length && a(p.old+1) == b(j+1)) {
          p.old += 1
          j += 1
      }
      p.last = record(arena,p.last,0,p.old-initial,separate)
      return j
  }
  private def finish(arena: ArrayBuffer[Run], last: Int): ArrayBuffer[Run] = {
      var i: Int = last
      val spans: ArrayBuffer[Run] = ArrayBuffer.empty[Run]
      while (i > 0) {
          spans.append(arena(i))
          i = arena(i).prev
      }
      flip(spans)
      return spans
  }
  private def myers(a: ArrayBuffer[String], b: ArrayBuffer[String], separate: Boolean): ArrayBuffer[Run] = {
      val limit: Int = a.length+b.length
      val base: Int = limit+1
      val best: ArrayBuffer[Path] = ArrayBuffer.empty[Path]
      for (i <- 0 until base*2+1) {
          best.append(Path(-2,0))
      }
      val arena: ArrayBuffer[Run] = ArrayBuffer.empty[Run]
      arena.append(Run(0,0,0))
      val first: Path = Path(-1,0)
      val origin: Int = advance(first,0,a,b,arena,separate)
      if (first.old+1 == a.length && origin+1 == b.length) {
          return finish(arena,first.last)
      }
      best(base) = first
      var lo: Int = -limit
      var hi: Int = limit
      for (d <- 1 until limit+1) {
          var k: Int = bigger(lo,-d)
          val stop: Int = smaller(hi,d)
          while (k <= stop) {
              val i: Int = base+k
              val rem: Path = best(i-1)
              val ins: Path = best(i+1)
              best(i-1) = Path(-2,0)
              val canR: Boolean = rem.old >= -1 && rem.old+1 < a.length
              val canA: Boolean = ins.old >= -1 && ins.old-k >= 0 && ins.old-k < b.length
              if (canR || canA) {
                  val take: Boolean = !canR || (canA && rem.old < ins.old)
                  val path: Path = Path((if (take) ins.old else rem.old+1),record(arena,(if (take) ins.last else rem.last),(if (take) 2 else 1),1,separate))
                  val j: Int = advance(path,k,a,b,arena,separate)
                  if (path.old+1 == a.length && j+1 == b.length) {
                      return finish(arena,path.last)
                  }
                  best(i) = path
                  if (path.old+1 == a.length) {
                      hi = smaller(hi,k-1)
                  }
                  if (j+1 == b.length) {
                      lo = bigger(lo,k+1)
                  }
              } else {
                  best(i) = Path(-2,0)
              }
              k += 2
          }
      }
      return ArrayBuffer.empty[Run]
  }

  private def ws(c: Int): Boolean = {
      return c >= 9 && c <= 13 || c == 32 || c == 160 || c == 0x1680 || c >= 0x2000 && c <= 0x200a || c == 0x2028 || c == 0x2029 || c == 0x202f || c == 0x205f || c == 0x3000 || c == 0xfeff
  }
  private def word(c: Int): Boolean = {
      return c >= 65 && c <= 90 || c >= 97 && c <= 122 || c >= 48 && c <= 57 || c == 95 || c == 0xad || c >= 0xc0 && c <= 0xd6 || c >= 0xd8 && c <= 0xf6 || c >= 0xf8 && c <= 0x2c6 || c >= 0x2c8 && c <= 0x2d7 || c >= 0x2de && c <= 0x2ff || c >= 0x1e00 && c <= 0x1eff
  }
  private def leading(s: String): String = {
      var i: Int = 0
      while (i < s.length && ws(code(s,i))) {
          i += 1
      }
      return cut(s,0,i)
  }
  private def trailing(s: String): String = {
      var i: Int = s.length
      while (i > 0 && ws(code(s,i-1))) {
          i -= 1
      }
      return cut(s,i,s.length)
  }
  private def trim(s: String): String = {
      val start: Int = leading(s).length
      return cut(s,start,bigger(start,s.length-trailing(s).length))
  }
  private def tokenize(op: String, s: String, options: js.Any): ArrayBuffer[String] = {
      val out: ArrayBuffer[String] = ArrayBuffer.empty[String]
      var i: Int = 0
      var start: Int = 0
      var prev: String = ""
      if (op == "diffLines" || op == "diffJson") {
          for (j <- 0 until s.length) {
              if (code(s,j) == 10) {
                  val end: Int = (if (j>start && code(s,j-1)==13) j-1 else j)
                  if (flag(options,"newlineIsToken")) {
                      if (end > start) {
                          out.append(cut(s,start,end))
                      }
                      out.append((if (flag(options,"stripTrailingCr")) "\n" else cut(s,end,j+1)))
                  } else {
                      out.append((if (flag(options,"stripTrailingCr") && end < j) cut(s,start,end)+"\n" else cut(s,start,j+1)))
                  }
                  start = j+1
              }
          }
          if (start < s.length) {
              out.append(cut(s,start,s.length))
          }
          return out
      }
      while (i < s.length) {
          val c: Int = code(s,i)
          start = i
          i += (if (c>=0xd800 && c<=0xdbff && i+1<s.length && code(s,i+1)>=0xdc00 && code(s,i+1)<=0xdfff) 2 else 1)
          if (op == "diffWords") {
              if (ws(c)) {
                  while (i < s.length && ws(code(s,i))) {
                      i += 1
                  }
                  prev = cut(s,start,i)
                  if (out.length == 0) {
                      out.append(prev)
                  } else {
                      out(out.length-1) = out(out.length-1) + prev
                  }
              } else {
                  if (word(c)) {
                      while (i < s.length && word(code(s,i))) {
                          i += 1
                      }
                  }
                  val t: String = cut(s,start,i)
                  if (prev != "" && out.length == 1 && out(0) == prev) {
                      out(0) = prev + t
                  } else {
                      out.append(prev+t)
                  }
                  prev = ""
              }
          } else {
              out.append(cut(s,start,i))
          }
      }
      return out
  }
  private def normal(op: String, tokens: ArrayBuffer[String], opts: js.Any): ArrayBuffer[String] = {
      val result: ArrayBuffer[String] = ArrayBuffer.empty[String]
      for (i <- 0 until tokens.length) {
          var s: String = tokens(i)
          if (op == "diffWords" || op == "diffLines" && flag(opts,"ignoreWhitespace") && (!flag(opts,"newlineIsToken") || !contains(s,"\n"))) {
              s = trim(s)
          } else {
              if (op == "diffLines" && flag(opts,"ignoreNewlineAtEof") && !flag(opts,"newlineIsToken") && ends(s,"\n")) {
                  s = cut(s,0,s.length-1)
              }
          }
          if (op == "diffJson") {
              s = replaceAll(replaceAll(s,",\n","\n"),",\r","\r")
          }
          result.append((if (flag(opts,"ignoreCase")) lower(s) else s))
      }
      return result
  }
  private def common(a: String, b: String, back: Boolean): Int = {
      var n: Int = 0
      while (n < a.length && n < b.length && code(a,(if (back) a.length-n-1 else n)) == code(b,(if (back) b.length-n-1 else n))) {
          n += 1
      }
      return n
  }
  private def overlap(a: String, b: String): Int = {
      var n: Int = smaller(a.length,b.length)
      while (n > 0 && cut(a,a.length-n,a.length) != cut(b,0,n)) {
          n -= 1
      }
      return n
  }
  private def clean(out: ArrayBuffer[Change], start: Int, del: Int, ins: Int, end: Int): Unit = {
      if (del >= 0 && ins >= 0) {
          val d: Change = out(del)
          val a: Change = out(ins)
          val dp: String = leading(d.value)
          val ap: String = leading(a.value)
          val ds: String = trailing(d.value)
          val asuf: String = trailing(a.value)
          if (start >= 0) {
              val n: Int = common(dp,ap,false)
              val s: Change = out(start)
              s.value = cut(s.value,0,s.value.length-ap.length)+cut(ap,0,n)
              d.value = cut(d.value,n,d.value.length)
              a.value = cut(a.value,n,a.value.length)
          }
          if (end >= 0) {
              val n: Int = common(ds,asuf,true)
              val e: Change = out(end)
              e.value = cut(asuf,asuf.length-n,asuf.length)+cut(e.value,asuf.length,e.value.length)
              d.value = cut(d.value,0,d.value.length-n)
              a.value = cut(a.value,0,a.value.length-n)
          }
      } else {
          if (ins >= 0) {
              if (start >= 0) {
                  val a: Change = out(ins)
                  a.value = cut(a.value,leading(a.value).length,a.value.length)
              }
              if (end >= 0) {
                  val e: Change = out(end)
                  e.value = cut(e.value,leading(e.value).length,e.value.length)
              }
          } else {
              if (del >= 0) {
                  val d: Change = out(del)
                  if (start >= 0 && end >= 0) {
                      val e: Change = out(end)
                      val s: Change = out(start)
                      val w: String = leading(e.value)
                      val a: Int = common(w,leading(d.value),false)
                      val b: Int = common(cut(w,a,w.length),trailing(d.value),true)
                      d.value = cut(d.value,a,d.value.length-b)
                      e.value = cut(e.value,w.length-b,e.value.length)
                      s.value = cut(s.value,0,s.value.length-b)
                  } else {
                      if (start >= 0) {
                          d.value = cut(d.value,overlap(trailing(out(start).value),leading(d.value)),d.value.length)
                      }
                      if (end >= 0) {
                          d.value = cut(d.value,0,d.value.length-overlap(trailing(d.value),leading(out(end).value)))
                      }
                  }
              }
          }
      }
  }
  private def changes(op: String, a: ArrayBuffer[String], b: ArrayBuffer[String], opts: js.Any): ArrayBuffer[Change] = {
      val spans: ArrayBuffer[Run] = myers(normal(op,a,opts),normal(op,b,opts),flag(opts,"oneChangePerToken"))
      val out: ArrayBuffer[Change] = ArrayBuffer.empty[Change]
      var old: Int = 0
      var pos: Int = 0
      for (i <- 0 until spans.length) {
          val r: Run = spans(i)
          var value: String = ""
          for (j <- 0 until r.count) {
              val s: String = (if (r.kind==1 || (op=="diffJson" && r.kind==0 && a(old+j).length>b(pos+j).length)) a(old+j) else b(pos+j))
              value += (if (op=="diffWords" && j>0) cut(s,leading(s).length,s.length) else s)
          }
          out.append(Change(r.kind,r.count,value))
          if (r.kind != 2) {
              old += r.count
          }
          if (r.kind != 1) {
              pos += r.count
          }
      }
      if (op == "diffWords" && !flag(opts,"oneChangePerToken")) {
          var start: Int = -1
          var del: Int = -1
          var ins: Int = -1
          for (i <- 0 until out.length) {
              if (out(i).kind == 2) {
                  ins = i
              } else {
                  if (out(i).kind == 1) {
                      del = i
                  } else {
                      clean(out,start,del,ins,i)
                      start = i
                      del = -1
                      ins = -1
                  }
              }
          }
          clean(out,start,del,ins,-1)
      }
      return out
  }
  private def fresh(): Hunk = {
      return Hunk(0,0,0,0,ArrayBuffer.empty[String])
  }
  private def emit(h: Hunk, k: Int, src: ArrayBuffer[String], at: Int, n: Int): Unit = {
      val mark: String = (if (k==2) "+" else (if (k==1) "-" else " "))
      for (i <- at until at+n) {
          val s: String = src(i)
          if (ends(s,"\n")) {
              h.lines.append(mark + cut(s,0,s.length-1))
          } else {
              h.lines.append(mark+s)
              h.lines.append("\\ No newline at end of file")
          }
      }
      if (k != 2) {
          h.oldLines += n
      }
      if (k != 1) {
          h.newLines += n
      }
  }
  private def create(input: js.Any, ctx: Int): Patch = {
      val meta: js.Any = obj()
      for (j <- 0 until 4) {
          val k: String = strAt("oldFileName|newFileName|oldHeader|newHeader",j)
          put(meta,k,get(input,k))
      }
      val p: Patch = Patch(meta,ArrayBuffer.empty[Hunk])
      val a: ArrayBuffer[String] = tokenize("diffLines",text(get(input,"before")),undefinedValue())
      val b: ArrayBuffer[String] = tokenize("diffLines",text(get(input,"after")),undefinedValue())
      val spans: ArrayBuffer[Run] = myers(a,b,false)
      var old: Int = 0
      var newer: Int = 0
      var kept: Int = 0
      var h: Hunk = fresh()
      for (i <- 0 until spans.length) {
          val r: Run = spans(i)
          if (r.kind == 0) {
              if (h.oldStart > 0) {
                  val close: Boolean = i+1==spans.length || r.count>ctx*2
                  emit(h,0,b,newer,(if (close) smaller(ctx,r.count) else r.count))
                  if (close) {
                      p.hunks.append(h)
                      h = fresh()
                  }
              }
              kept = r.count
          } else {
              if (h.oldStart == 0) {
                  val k: Int = smaller(ctx,kept)
                  h.oldStart = old+1-k
                  h.newStart = newer+1-k
                  emit(h,0,b,newer-k,k)
              }
              if (r.kind == 2) {
                  emit(h,r.kind,b,newer,r.count)
              } else {
                  emit(h,r.kind,a,old,r.count)
              }
          }
          if (r.kind != 2) {
              old += r.count
          }
          if (r.kind != 1) {
              newer += r.count
          }
      }
      if (h.oldStart > 0) {
          p.hunks.append(h)
      }
      return p
  }
  private def quotedName(s: String): String = {
      val bytes: ArrayBuffer[Int] = utf8(s.asInstanceOf[js.Dynamic].toWellFormed().asInstanceOf[String])
      var need: Boolean = false
      for (i <- 0 until bytes.length) {
          val c: Int = bytes(i)
          if (c < 32 || c > 126 || c == 34 || c == 92) {
              need = true
          }
      }
      if (!need) {
          return s
      }
      var out: String = "\""
      for (i <- 0 until bytes.length) {
          val c: Int = bytes(i)
          if (c >= 7 && c <= 13) {
              out += "\\"+cut("abtnvfr",c-7,c-6)
          } else {
              if (c == 34 || c == 92) {
                  out += "\\"+unit(c)
              } else {
                  if (c < 32 || c > 126) {
                      out += "\\"+unit(48+divide(c,64))+unit(48+divide(c,8)%8)+unit(48+c%8)
                  } else {
                      out += unit(c)
                  }
              }
          }
      }
      return out+"\""
  }
  private def endQuote(s: String): Int = {
      var i: Int = 1
      while (i < s.length) {
          val c: Int = code(s,i)
          if (c == 34) {
              return i
          }
          if (c == 92) {
              i += 1
          }
          i += 1
      }
      return -1
  }
  private def unquote(s: String): String = {
      if (!starts(s,"\"")) {
          return s
      }
      val stop: Int = endQuote(s)
      if (stop < 0) {
          return s
      }
      val bytes: ArrayBuffer[Int] = ArrayBuffer.empty[Int]
      val out = new StringBuilder
      def flush(): Unit = {
          if (bytes.nonEmpty) {
              val skip = if (bytes.startsWith(Seq(0xef, 0xbb, 0xbf))) 3 else 0
              out.append(decodeUtf8(bytes.drop(skip)))
              bytes.clear()
          }
      }
      var i: Int = 1
      while (i < stop) {
          val c: Int = code(s,i)
          i += 1
          if (c != 92) {
            flush()
              out.append(unit(c))
        } else {
              val v: Int = code(s,i)
              i += 1
              if (v >= 48 && v <= 55 && i+1 < stop) {
                  val x: Int = code(s,i)-48
                  val y: Int = code(s,i+1)-48
                  if (x < 0 || x > 7 || y < 0 || y > 7) {
                      return s
                  }
                  bytes.append(((v-48)*64+x*8+y)%256)
                  i += 2
              } else {
                  val j: Int = indexOf("abtnvfr",unit(v))
                  if (j < 0 && v != 34 && v != 92) {
                      return s
                  }
                  flush()
                  out.append(unit(if (j>=0) j+7 else v))
              }
          }
      }
      flush()
      return out.toString
  }
  private def formatPatch(p: Patch): String = {
      val old: js.Any = get(p.meta,"oldFileName")
      val newer: js.Any = get(p.meta,"newFileName")
      var out: String = ""
      if (defined(old) && defined(newer) && text(old) == text(newer)) {
          out += "Index: "+text(old)+"\n"
      }
      out += "===================================================================\n"
      if (defined(old) && defined(newer)) {
          for (i <- 0 until 2) {
              val k: String = (if (i==0) "old" else "new")
              val h: js.Any = get(p.meta,k+"Header")
              out += (if (i==0) "--- " else "+++ ")+quotedName(text(get(p.meta,k+"FileName")))+(if (defined(h) && text(h)!="") "\t"+text(h) else "")+"\n"
          }
      }
      for (i <- 0 until p.hunks.length) {
          val h: Hunk = p.hunks(i)
          out += "@@ -"+dec(h.oldStart-(if (h.oldLines==0) 1 else 0))+","+dec(h.oldLines)+" +"+dec(h.newStart-(if (h.newLines==0) 1 else 0))+","+dec(h.newLines)+" @@\n"
          for (j <- 0 until h.lines.length) {
              out += h.lines(j)+"\n"
          }
      }
      return out
  }
  private def writePatch(p: Patch, reverse: Boolean): js.Any = {
      val all: js.Any = arr(p.hunks.length)
      for (i <- 0 until p.hunks.length) {
          val h: Hunk = p.hunks(i)
          val x: js.Any = obj()
          if (reverse) {
              put(x,"oldLines",num(h.oldLines))
              put(x,"oldStart",num(h.oldStart))
              put(x,"newLines",num(h.newLines))
              put(x,"newStart",num(h.newStart))
          } else {
              put(x,"oldStart",num(h.oldStart))
              put(x,"oldLines",num(h.oldLines))
              put(x,"newStart",num(h.newStart))
              put(x,"newLines",num(h.newLines))
          }
          val data: js.Any = arr(h.lines.length)
          for (j <- 0 until h.lines.length) {
              assign(data,j,string(h.lines(j)))
          }
          put(x,"lines",data)
          assign(all,i,x)
      }
      put(p.meta,"hunks",all)
      return p.meta
  }
  private def readPatch(v: js.Any): Patch = {
      val meta: js.Any = obj()
      val ks: ArrayBuffer[String] = names(v)
      for (i <- 0 until ks.length) {
          put(meta,ks(i),get(v,ks(i)))
      }
      val p: Patch = Patch(meta,ArrayBuffer.empty[Hunk])
      val raw: js.Any = get(v,"hunks")
      for (i <- 0 until size(raw)) {
          val x: js.Any = item(raw,i)
          val h: Hunk = Hunk(integer(get(x,"oldStart")),integer(get(x,"oldLines")),integer(get(x,"newStart")),integer(get(x,"newLines")),ArrayBuffer.empty[String])
          val a: js.Any = get(x,"lines")
          for (j <- 0 until size(a)) {
              h.lines.append(text(item(a,j)))
          }
          p.hunks.append(h)
      }
      return p
  }
  private def fileLine(s: String): Boolean = {
      return s.length>3 && (starts(s,"---") || starts(s,"+++")) && (code(s,3)==32 || code(s,3)==9)
  }
  private def header(s: String): Boolean = {
      return starts(s,"diff --git ") || starts(s,"Index: ") || starts(s,"diff -r ")
  }
  private def fileMeta(meta: js.Any, s: String): Unit = {
      val parts: ArrayBuffer[String] = split(trim(cut(s,3,s.length)),"\t")
      val side: String = (if (starts(s,"---")) "old" else "new")
      val n: String = parts(0)
      put(meta,side+"FileName",string((if (starts(n,"\"")) unquote(n) else replaceAll(n,"\\\\","\\"))))
      put(meta,side+"Header",string((if (parts.length>1) trim(parts(1)) else "")))
  }
  private def gitNames(meta: js.Any, s: String): Unit = {
      var splitAt: Int = -1
      if (starts(s,"\"")) {
          splitAt = endQuote(s)+1
      } else {
          val q: Int = indexOf(s,"\"")
          if (q > 0) {
              splitAt = q-1
          } else {
              val spots: ArrayBuffer[Int] = ArrayBuffer.empty[Int]
              for (i <- 0 until bigger(0,s.length-2)) {
                  if (cut(s,i,i+3) == " b/") {
                      spots.append(i)
                  }
              }
              if (spots.length > 0 && starts(s,"a/")) {
                  splitAt = spots(divide(spots.length,2))
              }
          }
      }
      if (splitAt > 0 && splitAt+1 < s.length) {
          put(meta,"oldFileName",string(unquote(cut(s,0,splitAt))))
          put(meta,"newFileName",string(unquote(cut(s,splitAt+1,s.length))))
      }
  }
  private def extended(meta: js.Any, s: String): Unit = {
      for (i <- 0 until 8) {
          val tag: String = strAt("rename from |rename to |copy from |copy to |new file mode |deleted file mode |old mode |new mode ",i)
          if (starts(s,tag)) {
              val v: String = cut(s,tag.length,s.length)
              if (i < 4) {
                  put(meta,(if (i%2==0) "oldFileName" else "newFileName"),string((if (i%2==0) "a/" else "b/")+unquote(v)))
                  put(meta,(if (i<2) "isRename" else "isCopy"),boolVal(true))
              } else {
                  if (i < 6) {
                      put(meta,(if (i==4) "isCreate" else "isDelete"),boolVal(true))
                  }
                  put(meta,(if (i==5 || i==6) "oldMode" else "newMode"),string(v))
              }
              return
          }
      }
      if (starts(s,"Binary files ")) {
          put(meta,"isBinary",boolVal(true))
      }
  }
  private def parseRange(s: String): ArrayBuffer[Int] = {
      val cols: ArrayBuffer[String] = split(cut(s,1,s.length),",")
      val n: Int = (if (cols.length>1) parseInt(cols(1)) else 1)
      val result: ArrayBuffer[Int] = ArrayBuffer.empty[Int]
      result.append(parseInt(cols(0))+(if (n==0) 1 else 0))
      result.append(n)
      return result
  }
  private def parsePatch(s: String): ArrayBuffer[Patch] = {
      val body: ArrayBuffer[String] = split(s,"\n")
      val out: ArrayBuffer[Patch] = ArrayBuffer.empty[Patch]
      var i: Int = 0
      while (i < body.length) {
          val p: Patch = Patch(obj(),ArrayBuffer.empty[Hunk])
          put(p.meta,"hunks",undefinedValue())
          var seen: Boolean = false
          while (i < body.length && !fileLine(body(i)) && !starts(body(i),"@@ ") && !(seen && header(body(i)))) {
              val l: String = body(i)
              if (header(l)) {
                  seen = true
                  if (starts(l,"diff --git ")) {
                      put(p.meta,"isGit",boolVal(true))
                      gitNames(p.meta,cut(l,11,l.length))
                  } else {
                      put(p.meta,"index",string((if (starts(l,"Index: ")) trim(cut(l,7,l.length)) else cut(l,lastIndex(l," ")+1,l.length))))
                  }
              } else {
                  if (flag(p.meta,"isGit")) {
                      extended(p.meta,l)
                  }
              }
              i += 1
          }
          for (q <- 0 until 2) {
              if (i < body.length && fileLine(body(i))) {
                  fileMeta(p.meta,body(i))
                  i += 1
              }
          }
          ensure(defined(get(p.meta,"oldFileName")) == defined(get(p.meta,"newFileName")))
          while (i < body.length && !header(body(i)) && !fileLine(body(i)) && !starts(body(i),"===================================================================")) {
              val l: String = body(i)
              i += 1
              if (starts(l,"@@ ")) {
                  val cols: ArrayBuffer[String] = split(l," ")
                  ensure(cols.length>=4 && starts(cols(1),"-") && starts(cols(2),"+") && cols(3)=="@@")
                  val o: ArrayBuffer[Int] = parseRange(cols(1))
                  val n: ArrayBuffer[Int] = parseRange(cols(2))
                  val h: Hunk = Hunk(o(0),o(1),n(0),n(1),ArrayBuffer.empty[String])
                  var old: Int = 0
                  var newer: Int = 0
                  while (i<body.length && (old<h.oldLines || newer<h.newLines || starts(body(i),"\\"))) {
                      val line: String = body(i)
                      ensure(line!="" || i+1<body.length)
                      val mark: Int = (if (line=="") 32 else code(line,0))
                      ensure(mark==32 || mark==43 || mark==45 || mark==92)
                      old += (if (mark==32 || mark==45) 1 else 0)
                      newer += (if (mark==32 || mark==43) 1 else 0)
                      h.lines.append(line)
                      i += 1
                  }
                  if (old == 0 && h.oldLines == 1) {
                      h.oldLines = 0
                  }
                  if (newer == 0 && h.newLines == 1) {
                      h.newLines = 0
                  }
                  ensure(old==h.oldLines && newer==h.newLines)
                  ensure(i>=body.length || body(i)=="" || !contains("+ -",cut(body(i),0,1)) || fileLine(body(i)))
                  p.hunks.append(h)
              }
          }
          out.append(p)
      }
      return out
  }
  private def reversePatch(v: js.Any): js.Any = {
      val p: Patch = readPatch(v)
      val copy: Boolean = flag(v,"isCopy")
      if (copy) {
          val m: js.Any = obj()
          val ks: ArrayBuffer[String] = names(v)
          for (i <- 0 until ks.length) {
              val k: String = ks(i)
              if (k != "isCreate" && k != "isCopy" && k != "isRename") {
                  put(m,k,get(v,k))
              }
          }
          p.meta = m
      }
      for (i <- 0 until 8) {
          val k: String = strAt("oldFileName|oldHeader|newFileName|newHeader|oldMode|newMode|isCreate|isDelete",i)
          val other: String = strAt("newFileName|newHeader|oldFileName|oldHeader|newMode|oldMode|isDelete|isCreate",i)
          var v1: js.Any = get(v,other)
          if (flag(v,"isGit") && ends(k,"FileName") && defined(v1)) {
              val s: String = text(v1)
              if (starts(s,"a/") || starts(s,"b/")) {
                  v1 = string((if (starts(s,"a/")) "b/" else "a/")+cut(s,2,s.length))
              }
          }
          if (!copy || k != "isCreate") {
              put(p.meta,k,v1)
          }
      }
      if (copy) {
          put(p.meta,"newFileName",string("/dev/null"))
          put(p.meta,"newHeader",undefinedValue())
          put(p.meta,"isDelete",boolVal(true))
          p.hunks = ArrayBuffer.empty[Hunk]
      }
      for (i <- 0 until p.hunks.length) {
          val h: Hunk = p.hunks(i)
          val oldStart: Int = h.oldStart
          val oldLines: Int = h.oldLines
          h.oldStart = h.newStart
          h.oldLines = h.newLines
          h.newStart = oldStart
          h.newLines = oldLines
          for (j <- 0 until h.lines.length) {
              val l: String = h.lines(j)
              if (starts(l,"+") || starts(l,"-")) {
                  h.lines(j) = (if (starts(l,"+")) "-" else "+")+cut(l,1,l.length)
              }
          }
      }
      return writePatch(p,true)
  }
  private def convert(source: String, p: Patch): Unit = {
      val win: Boolean = contains(source,"\r\n")
      val unix: Boolean = contains(replaceAll(source,"\r\n",""),"\n")
      if (win == unix) {
          return
      }
      var pw: Boolean = false
      var pu: Boolean = false
      for (i <- 0 until p.hunks.length) {
          val lines: ArrayBuffer[String] = p.hunks(i).lines
          for (j <- 0 until lines.length) {
              if (!starts(lines(j),"\\")) {
                  if (ends(lines(j),"\r")) {
                      pw = true
                  } else {
                      if (j+1>=lines.length || !starts(lines(j+1),"\\")) {
                          pu = true
                      }
                  }
              }
          }
      }
      for (i <- 0 until p.hunks.length) {
          val lines: ArrayBuffer[String] = p.hunks(i).lines
          for (j <- 0 until lines.length) {
              val l: String = lines(j)
              if (win && !pw && !starts(l,"\\") && (j+1>=lines.length || !starts(lines(j+1),"\\"))) {
                  lines(j) = l+"\r"
              }
              if (unix && pw && !pu && ends(l,"\r")) {
                  lines(j) = cut(l,0,l.length-1)
              }
          }
      }
  }
  private def fits(h: Hunk, lines: ArrayBuffer[String], at: Int): Boolean = {
      var i: Int = at
      for (j <- 0 until h.lines.length) {
          val l: String = h.lines(j)
          if (!starts(l,"+") && !starts(l,"\\")) {
              if (i>=lines.length || lines(i)!=(if (l=="") "" else cut(l,1,l.length))) {
                  return false
              }
              i += 1
          }
      }
      return true
  }
  private def applyPatch(source: String, p: Patch): js.Any = {
      if (p.hunks.length == 0) {
          return string(source)
      }
      convert(source,p)
      val data: ArrayBuffer[String] = split(source,"\n")
      var ae: Boolean = false
      var re: Boolean = false
      var prev: Int = 0
      val last: ArrayBuffer[String] = p.hunks(p.hunks.length-1).lines
      for (i <- 0 until last.length) {
          val line: String = last(i)
          if (starts(line,"\\")) {
              ae = ae || prev==45
              re = re || prev==43
          }
          prev = (if (line=="") 32 else code(line,0))
      }
      val eof: Boolean = data(data.length-1)==""
      if (re && ae && eof || re && !ae && !eof || !re && ae && eof) {
          return boolVal(false)
      }
      if (re && !ae) {
          pop(data)
      }
      if (!re && ae) {
          data.append("")
      }
      var minLine: Int = 0
      var offset: Int = 0
      val res: ArrayBuffer[String] = ArrayBuffer.empty[String]
      for (i <- 0 until p.hunks.length) {
          val h: Hunk = p.hunks(i)
          val maxLine: Int = data.length-h.oldLines
          if (maxLine < minLine) {
              return boolVal(false)
          }
          val first: Int = bigger(minLine,smaller(maxLine,h.oldStart+offset-1))
          var pos: Int = -1
          var d: Int = 0
          while (pos<0 && d<=bigger(first-minLine,maxLine-first)) {
              if (first+d<=maxLine && fits(h,data,first+d)) {
                  pos = first+d
              } else {
                  if (d>0 && first-d>=minLine && fits(h,data,first-d)) {
                      pos = first-d
                  }
              }
              d += 1
          }
          if (pos<0) {
              return boolVal(false)
          }
          for (j <- minLine until pos) {
              res.append(data(j))
          }
          var cursor: Int = pos
          var ctx: Int = 0
          for (j <- 0 until h.lines.length) {
              val l: String = h.lines(j)
              val op: Int = (if (l=="") 32 else code(l,0))
              if (op==43) {
                  res.append(cut(l,1,l.length))
                  ctx = 0
              }
              if (op==45) {
                  cursor += 1
                  ctx = 0
              }
              if (op==32) {
                  res.append(data(cursor))
                  cursor += 1
                  ctx += 1
              }
          }
          for (j <- 0 until ctx) {
              pop(res)
          }
          minLine = cursor-ctx
          offset = pos+1-h.oldStart
      }
      for (j <- minLine until data.length) {
          res.append(data(j))
      }
      return string(join(res,"\n"))
  }
  private def canonical(v: js.Any): js.Any = {
      if (isArray(v)) {
          val a: js.Any = arr(size(v))
          for (i <- 0 until size(v)) {
              assign(a,i,canonical(item(v,i)))
          }
          return a
      }
      if (isObject(v)) {
          val m: js.Any = dictionary()
          val ks: ArrayBuffer[String] = names(v)
          sortNames(ks)
          for (i <- 0 until ks.length) {
              put(m,ks(i),canonical(get(v,ks(i))))
          }
          return m
      }
      return v
  }
  private def doc(v: js.Any, op: String): String = {
      if (op=="diffJson" && !isString(v)) {
          return pretty(canonical(v))
      }
      return text(v)
  }
  private def writeChange(kind: Int, n: Int, v: js.Any): js.Any = {
      val x: js.Any = obj()
      put(x,"count",num(n))
      put(x,"added",boolVal(kind==2))
      put(x,"removed",boolVal(kind==1))
      put(x,"value",v)
      return x
  }
  private def arrayKeys(v: js.Any): ArrayBuffer[String] = {
      val keys: ArrayBuffer[String] = ArrayBuffer.empty[String]
      for (i <- 0 until size(v)) {
          keys.append(compact(item(v,i)))
      }
      return keys
  }
  private def execute(input: js.Any): js.Any = {
      val op: String = text(get(input,"op"))
      val a: js.Any = get(input,"before")
      val b: js.Any = get(input,"after")
      val opt: js.Any = get(input,"options")
      val arg: js.Any = get(input,"patch")
      if (op=="diffArrays") {
          val spans: ArrayBuffer[Run] = myers(arrayKeys(a),arrayKeys(b),flag(opt,"oneChangePerToken"))
          val all: js.Any = arr(spans.length)
          var old: Int = 0
          var newer: Int = 0
          for (i <- 0 until spans.length) {
              val r: Run = spans(i)
              val v: js.Any = arr(r.count)
              for (j <- 0 until r.count) {
                  assign(v,j,(if (r.kind==1) item(a,old+j) else item(b,newer+j)))
              }
              assign(all,i,writeChange(r.kind,r.count,v))
              old += (if (r.kind==2) 0 else r.count)
              newer += (if (r.kind==1) 0 else r.count)
          }
          return all
      }
      if (op=="diffChars" || op=="diffWords" || op=="diffLines" || op=="diffJson") {
          val rows: ArrayBuffer[Change] = changes(op,tokenize(op,doc(a,op),opt),tokenize(op,doc(b,op),opt),opt)
          val all: js.Any = arr(rows.length)
          for (i <- 0 until rows.length) {
              val c: Change = rows(i)
              assign(all,i,writeChange(c.kind,c.count,string(c.value)))
          }
          return all
      }
      if (op=="structuredPatch" || op=="createTwoFilesPatch") {
          val ctx: js.Any = get(opt,"context")
          val p: Patch = create(input,(if (defined(ctx)) integer(ctx) else 4))
          return (if (op=="structuredPatch") writePatch(p,false) else string(formatPatch(p)))
      }
      if (op=="parsePatch") {
          val ps: ArrayBuffer[Patch] = parsePatch(text(arg))
          val all: js.Any = arr(ps.length)
          for (i <- 0 until ps.length) {
              assign(all,i,writePatch(ps(i),false))
          }
          return all
      }
      if (op=="reversePatch") {
          if (isArray(arg)) {
              val n: Int = size(arg)
              val all: js.Any = arr(n)
              for (i <- 0 until n) {
                  assign(all,n-i-1,reversePatch(item(arg,i)))
              }
              return all
          }
          return reversePatch(arg)
      }
      if (op=="applyPatch") {
          if (isString(arg)) {
              val ps: ArrayBuffer[Patch] = parsePatch(text(arg))
              ensure(ps.length==1)
              return applyPatch(text(a),ps(0))
          }
          if (isArray(arg)) {
              ensure(size(arg)==1)
              return applyPatch(text(a),readPatch(item(arg,0)))
          }
          return applyPatch(text(a),readPatch(arg))
      }
      ensure(false)
      return undefinedValue()
  }
  @JSExportTopLevel("run")
  def run(input: js.Any):js.Any=execute(input)
}
