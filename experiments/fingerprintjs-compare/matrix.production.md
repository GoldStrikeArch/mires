# FingerprintJS compiler comparison matrix

Generated 2026-08-15T08:50:26.474Z.

| Implementation | Profile | Complete | ESM raw | ESM min | IIFE min | IIFE gzip-9 | IIFE brotli-11 | Runtime bytes |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| TypeScript oracle | production | no | 129311 | — | 36746 | 16178 | 14239 | 0 | 
| ReScript | production | yes | 96158 | 42198 | 42283 | 17453 | 15422 | — | 
| Melange 7.0.1 | production | yes | 107682 | 49769 | 49856 | 18840 | 16655 | — | 
| js_of_ocaml | production | yes | 358357 | 136383 | 136140 | 47988 | 41870 | — | 
| Compact Rust | production | yes | 520272 | 159764 | 159745 | 43910 | 33627 | 17092 | 
| Dart2js -O2 (production) | production | yes | 166823 | 151444 | 151168 | 52147 | 44842 | — | 
| Scala.js fullLink (production) | production | yes | 609808 | 229011 | 228513 | 67338 | 56734 | — | 
| Elm compiler-aware minifier | production | yes | 82723 | 53884 | 53949 | 21964 | 19616 | — | 
| Kotlin/JS Gradle production | production | yes | 221362 | 77274 | 77057 | 28817 | 25514 | — | 

