const wide = 1n
let cached
cached ||= wide
cached ??= 0
globalThis.__rcgjs_post_es2018 = cached?.toString()
