fun main() {
    installPublicExports()
}

private fun installPublicExports() {
    val exp = obj()
    val loadFn = ::load
    val hashFn = ::hashComponents
    val debugFn = ::componentsToDebugString
    jsSet(exp, "componentsToDebugString", debugFn)
    jsSet(exp, "getFullscreenElement", ::getFullscreenElement)
    jsSet(exp, "getUnstableAudioFingerprint", ::getUnstableAudioFingerprint)
    jsSet(exp, "getUnstableCanvasFingerprint", ::getUnstableCanvasFingerprint)
    jsSet(exp, "getUnstableHardwareConcurrency", ::getUnstableHardwareConcurrency)
    jsSet(exp, "getUnstableScreenFrame", ::getUnstableScreenFrame)
    jsSet(exp, "getUnstableScreenResolution", ::getUnstableScreenResolution)
    jsSet(exp, "getWebGLContext", ::getWebGLContext)
    jsSet(exp, "hashComponents", hashFn)
    jsSet(exp, "isAndroid", ::isAndroid)
    jsSet(exp, "isChromium", ::isChromium)
    jsSet(exp, "isDesktopWebKit", ::isDesktopWebKit)
    jsSet(exp, "isEdgeHTML", ::isEdgeHTML)
    jsSet(exp, "isGecko", ::isGecko)
    jsSet(exp, "isSamsungInternet", ::isSamsungInternet)
    jsSet(exp, "isTrident", ::isTrident)
    jsSet(exp, "isWebKit", ::isWebKit)
    jsSet(exp, "load", loadFn)
    jsSet(exp, "loadSources", ::loadSources)
    jsSet(exp, "murmurX64Hash128", ::murmurX64Hash128)
    jsSet(exp, "prepareForSources", ::prepareForSources)
    jsSet(exp, "sources", builtinSources)
    jsSet(exp, "transformSource", ::transformSource)
    jsSet(exp, "withIframe", ::withIframe)
    val defaultExport = obj()
    jsSet(defaultExport, "load", loadFn)
    jsSet(defaultExport, "hashComponents", hashFn)
    jsSet(defaultExport, "componentsToDebugString", debugFn)
    jsSet(exp, "default", defaultExport)
    installGlobalExports(exp)
}
