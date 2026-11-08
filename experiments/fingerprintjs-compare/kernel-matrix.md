# FingerprintJS production kernel matrix

Generated 2026-08-15T08:13:06.783Z.

This is an isolated Node.js diagnostic for the exported MurmurHash3 path. It complements, but does not replace, the protected browser benchmark.

| Implementation | Kernel p50 | ASCII p50 | Unicode p50 | ASCII p95 | Unicode p95 | Module import |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ReScript | 0.98× | 1.00× | 0.96× | 1.04× | 0.93× | 0.90× | 
| Scala.js fullLink (production) | 1.00× | 1.00× | 1.00× | 0.96× | 0.96× | 4.66× | 
| Melange 7.0.1 | 1.03× | 1.01× | 1.05× | 1.05× | 1.26× | 1.37× | 
| Compact Rust | 1.46× | 1.43× | 1.48× | 1.57× | 1.46× | 3.14× | 
| Dart2js -O2 (production) | 1.77× | 1.53× | 2.06× | 1.53× | 2.00× | 4.73× | 
| js_of_ocaml | 2.09× | 2.08× | 2.10× | 1.96× | 1.98× | 2.68× | 
| Elm compiler-aware minifier | 8.67× | 8.87× | 8.46× | 8.55× | 8.01× | 1.49× | 
| Kotlin/JS Gradle production | 62.44× | 62.80× | 62.07× | 60.08× | 57.55× | 1.88× | 

