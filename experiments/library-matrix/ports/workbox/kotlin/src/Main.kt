import kotlin.coroutines.*
import kotlin.js.Promise

external interface ManifestEntry { val path:String; val revision:String? }
external interface WorkerEvent { fun waitUntil(task:Promise<Any?>) }
external interface Step {
    val phase:String?
    val name:String?
    val source:String?
    val manifest:Array<ManifestEntry>?
    val path:String?
    val method:String?
    val request:WebRequest?
    val event:WorkerEvent?
    val message:String?
    val steps:Array<Step>?
}
@JsName("URL")
external class WebUrl(path:String,base:String) {
    val href:String
    var hash:String
    var pathname:String
    val searchParams:QueryParams
}
external interface QueryParams {
    fun keys():JsAnyIterator
    fun set(key:String,value:String)
    fun delete(key:String)
}
external interface JsAnyIterator
@JsName("Array") external object JsArrayConstructor {fun from(keys:JsAnyIterator):Array<String>}
@JsName("Request") external class WebRequest(url:String, init:kotlin.js.Json = definedExternally) {val url:String;val method:String}
external interface WebResponse {
    val status:Int
    fun clone():WebResponse
    fun text():Promise<String>
}
external interface WebCache {
    fun match(url:String):Promise<WebResponse?>
    fun put(url:String,response:WebResponse):Promise<Unit>
    fun keys():Promise<Array<WebRequest>>
    fun delete(url:String):Promise<Boolean>
}
external interface Storage {
    fun open(name:String):Promise<WebCache>
    fun delete(name:String):Promise<Boolean>
}
external interface CryptoProvider {fun randomUUID():String}
external interface LocationInfo {val href:String}
@JsName("caches") external val storage:Storage
@JsName("crypto") external val cryptoProvider:CryptoProvider
@JsName("location") external val locationInfo:LocationInfo
@JsName("fetch") external fun networkFetch(request:WebRequest):Promise<WebResponse>
@JsName("skipWaiting") external fun skipWaiting():Promise<Unit>

private suspend fun <T> Promise<T>.await():T = suspendCoroutine { result ->
    then({value -> result.resume(value)}, {error -> result.resumeWithException(error)})
}
private fun <T> launch(task:suspend ()->T):Promise<T> = Promise { resolve,reject ->
    task.startCoroutine(object:Continuation<T> {
        override val context:CoroutineContext = EmptyCoroutineContext
        override fun resumeWith(result:Result<T>) { result.fold(resolve,reject) }
    })
}
private fun manifest(items:Array<ManifestEntry>,base:String):Map<String,String> {
    val entries=linkedMapOf<String,String>()
    for(item in items){
        val url=WebUrl(item.path,base)
        val original=url.href
        if(!item.revision.isNullOrEmpty())url.searchParams.set("__WB_REVISION__",item.revision!!)
        val old=entries[original]
        if(old!=null && old!=url.href)error("conflicting revisions")
        entries[original]=url.href
    }
    return entries
}
private fun lookup(entries:Map<String,String>,path:String,base:String):String? {
    val url=WebUrl(path,base)
    url.hash=""
    entries[url.href]?.let{return it}
    for(key in JsArrayConstructor.from(url.searchParams.keys())){
        if(key=="fbclid" || key.startsWith("utm_")) url.searchParams.delete(key)
    }
    entries[url.href]?.let{return it}
    val pathname=url.pathname
    if(pathname.endsWith("/")){
        url.pathname=pathname+"index.html"
        entries[url.href]?.let{return it}
    }
    url.pathname=pathname+".html"
    return entries[url.href]
}
private suspend fun network(source:String,base:String):WebResponse {
    if(source=="offline")error("offline")
    return networkFetch(WebRequest(WebUrl(source+".mjs",base).href)).await()
}
private suspend fun phase(step:Step,name:String,items:Array<ManifestEntry>,base:String):Any? {
    val entries=manifest(items,base)
    val cache=storage.open(name).await()
    val source=step.source ?: "alpha"
    when(step.phase){
        "install"->{
            val updated=mutableListOf<String>()
            val unchanged=mutableListOf<String>()
            for((original,key) in entries){
                if(cache.match(key).await()!=null)unchanged+=original.removePrefix(base)
                else {
                    val response=network(source,base)
                    if(response.status>=400)error("bad-precaching-response")
                    cache.put(key,response.clone()).await()
                    updated+=original.removePrefix(base)
                }
            }
            return arrayOf(updated.toTypedArray(),unchanged.toTypedArray())
        }
        "activate"->{
            val removed=mutableListOf<String>()
            for(request in cache.keys().await()){
                if(request.url !in entries.values){
                    cache.delete(request.url).await()
                    removed+=request.url.removePrefix(base)
                }
            }
            return removed.sorted().toTypedArray()
        }
        "message"->{
            val matched=step.message=="SKIP_WAITING"
            if(matched)skipWaiting().await()
            return matched
        }
    }
    val request = step.request ?: WebRequest(WebUrl(step.path!!, base).href,
        kotlin.js.json("method" to (step.method?.takeIf { it.isNotEmpty() } ?: "GET")))
    if (request.method != "GET") return null
    val path = request.url
    val key=lookup(entries,path,base)
    val relative=path.removePrefix(base)
    return try {
        val response=when {
            key!=null->cache.match(key).await() ?: network(source,base)
            relative.startsWith("network/")->network(source,base)
            relative.startsWith("first/") || relative.startsWith("only/")->{
                val runtime=storage.open(name+"-runtime").await()
                runtime.match(path).await() ?: run {
                    if(relative.startsWith("only/"))error("no-response")
                    val fetched=network(source,base)
                    if(fetched.status==200)runtime.put(path,fetched.clone()).await()
                    fetched
                }
            }
            else->return null
        }
        arrayOf(response.status,response.text().await())
    } catch(e:Throwable){arrayOf("no-response")}
}
@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input:Step):Promise<Any?> {
    val base=WebUrl("experiments/library-matrix/libraries/workbox/",locationInfo.href).href
    if(input.phase!=null){
        val task=launch {phase(input,input.name!!,input.manifest ?: emptyArray(),base)}
        input.event?.waitUntil(task)
        return task
    }
    return launch {
        val name="mires-workbox-"+cryptoProvider.randomUUID()
        val out=mutableListOf<Any?>()
        var items=emptyArray<ManifestEntry>()
        try {
            for(step in input.steps!!){
                items=step.manifest ?: items
                out.add(phase(step,name,items,base))
            }
            out.toTypedArray()
        } finally {storage.delete(name).await();storage.delete(name+"-runtime").await()}
    }
}
