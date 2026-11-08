# FingerprintJS compiler comparison matrix

Generated 2026-08-15T08:50:26.474Z.

| Implementation | Profile | Complete | ESM raw | ESM min | IIFE min | IIFE gzip-9 | IIFE brotli-11 | Runtime bytes |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| TypeScript oracle | production | no | 129311 | — | 36746 | 16178 | 14239 | 0 | 
| Idiomatic Rust | baseline | yes | 1087270 | 332381 | 332361 | 73917 | 59297 | 21173 | 
| ReScript | production | yes | 96158 | 42198 | 42283 | 17453 | 15422 | — | 
| Melange 7.0.1 | production | yes | 107682 | 49769 | 49856 | 18840 | 16655 | — | 
| js_of_ocaml | production | yes | 358357 | 136383 | 136140 | 47988 | 41870 | — | 
| Compact Rust | production | yes | 520272 | 159764 | 159745 | 43910 | 33627 | 17092 | 
| Hybrid Rust | experimental | yes | 882461 | 269652 | 269643 | 68646 | 56222 | — | 
| Dart2js -O1 (readable-first) | readable | yes | 435032 | 252175 | 251899 | 64816 | 53633 | — | 
| Dart2js -O2 (production) | production | yes | 166823 | 151444 | 151168 | 52147 | 44842 | — | 
| Scala.js fastLink (readable-first) | readable | yes | 995265 | 485633 | 485126 | 91364 | 73517 | — | 
| Scala.js fullLink (production) | production | yes | 609808 | 229011 | 228513 | 67338 | 56734 | — | 
| Elm standard minifier | readable | yes | 143839 | 57914 | 57980 | 22992 | 20488 | — | 
| Elm compiler-aware minifier | production | yes | 82723 | 53884 | 53949 | 21964 | 19616 | — | 
| Kotlin/JS readable packaging | readable | yes | 213782 | 74896 | 74653 | 27409 | 24225 | 108234 | 
| Kotlin/JS js/app packaging proxy | proxy | yes | 210806 | 74246 | 74002 | 27176 | 23989 | 108234 | 
| Kotlin/JS Gradle production | production | yes | 221362 | 77274 | 77057 | 28817 | 25514 | — | 

