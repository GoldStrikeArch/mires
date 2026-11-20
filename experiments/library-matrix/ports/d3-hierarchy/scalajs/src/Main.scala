package hierarchy

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel
import scala.collection.mutable

@js.native trait Datum extends js.Object:
  val id: js.UndefOr[String] = js.native
  val parentId: js.UndefOr[String] = js.native
  val value: js.UndefOr[Double] = js.native
  val children: js.UndefOr[js.Array[Datum]] = js.native

@js.native trait Input extends js.Object:
  val op: String = js.native
  val source: String = js.native
  val data: Datum = js.native
  val rows: js.Array[Datum] = js.native
  val width: Double = js.native
  val height: Double = js.native
  val padding: Double = js.native
  val sort: Boolean = js.native
  val round: Boolean = js.native
  val nodeSize: Boolean = js.native
  val tile: String = js.native

object Main:
  private def slash(s:String,i:Int):Boolean =
    if i<0 || i>=s.length || s(i)!='/' then return false
    var k=i
    while k>0 && s(k-1)=='\\' do k-=1
    (i-k)%2==0
  private def normalize(s:String):String =
    val path=if slash(s,s.length-1) && !slash(s,s.length-2) then s.dropRight(1) else s
    if path.startsWith("/") then path else "/"+path
  private def parentPath(s:String):String =
    if s.length<2 then return ""
    var i=s.length-1
    while i>1 && !slash(s,i) do i-=1
    s.substring(0,i)
  private def walk(nodes:Array[Node],root:Int,post:Boolean):Array[Int] =
    val out=mutable.ArrayBuffer.empty[Int]
    val stack=mutable.ArrayBuffer(root)
    while stack.nonEmpty do
      val v=stack.remove(stack.size-1)
      out+=v
      if post then stack++=nodes(v).kids else stack++=nodes(v).kids.reverseIterator
    if post then out.reverseIterator.toArray else out.toArray
  private def read(input:Input):(Array[Node],Int) =
    val nodes=mutable.ArrayBuffer.empty[Node]
    if input.source!="id" && input.source!="path" then
      def nested(d:Datum,p:Int):Int =
        val i=nodes.size
        val n=new Node
        nodes+=n
        n.id=d.id.getOrElse("");n.value=d.value.getOrElse(0.0);n.parent=p
        n.kids=d.children.toOption.map(_.iterator.map(c=>nested(c,i)).toArray).getOrElse(Array.empty[Int])
        i
      nested(input.data,-1)
      return (nodes.toArray,0)
    val paths=input.source=="path"
    val byId=mutable.Map.empty[String,Int]
    val parents=mutable.ArrayBuffer.empty[String]
    val count=input.rows.length
    for (d,i)<-input.rows.iterator.zipWithIndex do
      val n=new Node
      n.id=if paths then normalize(d.id.getOrElse("")) else d.id.getOrElse("")
      n.value=d.value.getOrElse(0.0)
      nodes+=n
      if n.id.nonEmpty then byId(n.id)=if byId.contains(n.id) then -1 else i
      parents+=(if paths then parentPath(n.id) else d.parentId.getOrElse(""))
    var i=0
    while paths && i<parents.size do
      val p=parents(i);i+=1
      if p.nonEmpty && !byId.contains(p) then
        byId(p)=nodes.size
        val n=new Node;n.id=p;nodes+=n;parents+=parentPath(p)
    val kids=Array.fill(nodes.size)(mutable.ArrayBuffer.empty[Int])
    var root= -1
    for (p,v)<-parents.iterator.zipWithIndex do
      if p.isEmpty then
        require(root<0);root=v
      else
        val id=byId.getOrElse(p,-1)
        require(id>=0);nodes(v).parent=id;kids(id)+=v
    require(root>=0)
    val ns=nodes.toArray
    for j<-ns.indices do ns(j).kids=kids(j).toArray
    var expected=ns.length
    while paths && root>=count && ns(root).kids.length==1 do
      root=ns(root).kids(0);expected-=1
    ns(root).parent= -1
    require(walk(ns,root,false).length==expected)
    (ns,root)
  @JSExportTopLevel("run")
  def run(input:Input):js.Array[js.Array[js.Any]] =
    val (nodes,root)=read(input)
    for i<-walk(nodes,root,true) do
      val n=nodes(i)
      if n.value == 0.0 then n.value = 0.0
      for k<-n.kids.reverseIterator do
        val c=nodes(k);n.value+=c.value;n.height=n.height.max(c.height+1)
      if input.sort then n.kids=n.kids.sortBy(i => -nodes(i).value)
    val pre=walk(nodes,root,false);val post=walk(nodes,root,true)
    for i<-pre do
      val n=nodes(i)
      if n.parent>=0 then n.depth=nodes(n.parent).depth+1
    val w=input.width;val h=input.height;val p=input.padding
    input.op match
      case "tree" => Geometry.tidy(nodes,pre,post,w,h,input.nodeSize)
      case "cluster" => Geometry.cluster(nodes,pre,post,w,h,input.nodeSize)
      case "pack" => Geometry.pack(nodes,pre,post,w,h,p)
      case "treemap" | "partition" => Geometry.rectangles(nodes,pre,w,h,p,input.op=="partition",input.tile,input.round)
      case "iterator" =>
        pre(0)=root
        var j=1
        for i<-pre.indices do
          for c<-nodes(pre(i)).kids do
            pre(j)=c;j+=1
      case "hierarchy" | "stratify" => ()
      case _ => throw new IllegalArgumentException("operation")
    val output=new js.Array[js.Array[js.Any]]
    for i<-pre do
      val n=nodes(i)
      val row=js.Array[js.Any](n.id,if n.parent<0 then null else nodes(n.parent).id,n.depth,n.height,n.value)
      input.op match
        case "tree" | "cluster" => row.push(n.x,n.y)
        case "pack" => row.push(n.x,n.y,n.r)
        case "treemap" | "partition" => row.push(n.x0,n.y0,n.x1,n.y1)
        case _ => ()
      output.push(row)
    output
