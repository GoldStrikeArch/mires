external interface Datum {
    val id: String?
    val parentId: String?
    val value: Double?
    val children: Array<Datum>?
}
external interface Input {
    val op: String
    val source: String
    val data: Datum?
    val rows: Array<Datum>?
    val width: Double
    val height: Double
    val padding: Double
    val sort: Boolean
    val round: Boolean
    val nodeSize: Boolean
    val tile: String
}
private fun slash(s:String, i:Int):Boolean {
    if(i !in s.indices || s[i]!='/') return false
    var k=i
    while(k>0 && s[k-1]=='\\') k--
    return (i-k)%2==0
}
private fun normalize(s:String):String {
    val path=if(slash(s,s.length-1) && !slash(s,s.length-2)) s.dropLast(1) else s
    return if(path.startsWith("/")) path else "/"+path
}
private fun parentPath(s:String):String {
    if(s.length<2) return ""
    var i=s.length-1
    while(i>1 && !slash(s,i)) i--
    return s.substring(0,i)
}
private fun walk(nodes:Array<Node>, root:Int, post:Boolean):IntArray {
    val order=mutableListOf<Int>()
    val stack=mutableListOf(root)
    while(stack.isNotEmpty()){
        val i=stack.removeAt(stack.lastIndex)
        order.add(i)
        val kids=nodes[i].kids
        if(post) for(c in kids) stack.add(c) else for(j in kids.indices.reversed()) stack.add(kids[j])
    }
    if(post) order.reverse()
    return order.toIntArray()
}
private fun read(input:Input):Pair<Array<Node>,Int>{
    val nodes=mutableListOf<Node>()
    if(input.source!="id" && input.source!="path"){
        fun nested(d:Datum,p:Int):Int{
            val i=nodes.size
            val n=Node()
            nodes.add(n)
            n.id=d.id?:""
            n.value=d.value?:0.0
            n.parent=p
            n.kids=d.children?.map { nested(it,i) }?.toIntArray()?:intArrayOf()
            return i
        }
        nested(input.data!!,-1)
        return Pair(nodes.toTypedArray(),0)
    }
    val rows=input.rows!!
    val paths=input.source=="path"
    val byId=mutableMapOf<String,Int>()
    val parents=mutableListOf<String>()
    for((i,d) in rows.withIndex()){
        val n=Node()
        n.id=if(paths) normalize(d.id?:"") else d.id?:""
        n.value=d.value?:0.0
        nodes.add(n)
        if(n.id!="") byId[n.id]=if(n.id in byId) -1 else i
        parents.add(if(paths) parentPath(n.id) else d.parentId?:"")
    }
    var i=0
    while(paths && i<parents.size){
        val p=parents[i++]
        if(p!="" && p !in byId){
            byId[p]=nodes.size
            val n=Node();n.id=p;nodes.add(n);parents.add(parentPath(p))
        }
    }
    var root=-1
    val children=Array(nodes.size){ mutableListOf<Int>() }
    for((v,parent) in parents.withIndex()){
        if(parent==""){require(root<0);root=v}
        else {
            val p=byId[parent]?:-1
            require(p>=0)
            nodes[v].parent=p
            children[p].add(v)
        }
    }
    require(root>=0)
    val ns=nodes.toTypedArray()
    for(j in ns.indices) ns[j].kids=children[j].toIntArray()
    var expected=ns.size
    while(paths && root>=rows.size && ns[root].kids.size==1) {root=ns[root].kids[0];expected--}
    ns[root].parent=-1
    require(walk(ns,root,false).size==expected)
    return Pair(ns,root)
}
@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input:Input):dynamic{
    val (nodes,root)=read(input)
    for(i in walk(nodes,root,true)){
        val n=nodes[i]
        if (n.value == 0.0) n.value = 0.0
        for(j in n.kids.indices.reversed()){
            val c=nodes[n.kids[j]];n.value+=c.value;n.height=maxOf(n.height,c.height+1)
        }
        if(input.sort)n.kids=n.kids.sortedByDescending { nodes[it].value }.toIntArray()
    }
    val pre=walk(nodes,root,false);val post=walk(nodes,root,true)
    for(i in pre){val n=nodes[i];if(n.parent>=0)n.depth=nodes[n.parent].depth+1}
    val w=input.width;val h=input.height;val p=input.padding
    when(input.op){
        "tree" -> tidy(nodes,pre,post,w,h,input.nodeSize)
        "cluster" -> cluster(nodes,pre,post,w,h,input.nodeSize)
        "pack" -> pack(nodes,pre,post,w,h,p)
        "partition","treemap" -> rectangles(nodes,pre,w,h,p,input.op=="partition",input.tile,input.round)
        "iterator" -> {pre[0]=root;var j=1;for(i in pre.indices) for(c in nodes[pre[i]].kids) pre[j++]=c}
        "hierarchy","stratify" -> Unit
        else -> error("unknown operation")
    }
    return pre.map {
        val n=nodes[it];val parent=if(n.parent>=0) nodes[n.parent].id else null
        val row=mutableListOf<Any?>(n.id,parent,n.depth,n.height,n.value)
        when(input.op){
            "tree","cluster" -> row.addAll(listOf(n.x,n.y))
            "pack" -> row.addAll(listOf(n.x,n.y,n.r))
            "treemap","partition" -> row.addAll(listOf(n.x0,n.y0,n.x1,n.y1))
        };row.toTypedArray()
    }.toTypedArray()
}
