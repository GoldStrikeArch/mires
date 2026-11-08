# FingerprintJS production runtime matrix

Generated 2026-08-15T08:50:41.592Z.

Rows are ordered by the runtime gate, then end-to-end total p95. PASS requires both aggregate and every measured total-p95 cell to remain within 5% of the oracle. Bundle size is secondary and never changes the ordering.

| Implementation | Gate | Cells | Total p95 | Worst total cell | Compiler overhead p95 | Eval p95 | Immediate get p95 | Lazy hash p95 | Event-loop p95 | Heap delta p50 | IIFE gzip-9 | Gzip vs oracle |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Melange 7.0.1 | PASS | 9 | 1.00× | 1.02× | 1.37× | 1.34× | 2.09× | 0.93× | 1.05× | 1.10× | 18840 | 1.16× | 
| ReScript | PASS | 9 | 1.01× | 1.03× | 1.39× | 1.31× | 2.02× | 1.02× | 1.04× | 0.98× | 17453 | 1.08× | 
| Scala.js fullLink (production) | FAIL | 9 | 1.04× | 1.14× | 1.90× | 3.78× | 2.09× | 0.87× | 1.07× | 2.17× | 67338 | 4.16× | 
| js_of_ocaml | FAIL | 9 | 1.05× | 1.12× | 1.74× | 2.61× | 1.68× | 1.20× | 1.14× | 4.13× | 47988 | 2.97× | 
| Elm compiler-aware minifier | FAIL | 9 | 1.08× | 1.13× | 1.97× | 1.57× | 0.97× | 5.02× | 1.23× | — | 21964 | 1.36× | 
| Compact Rust | FAIL | 9 | 1.09× | 1.12× | 2.04× | 2.19× | 2.63× | 1.47× | 1.31× | — | 43910 | 2.71× | 
| Dart2js -O2 (production) | FAIL | 9 | 1.10× | 1.15× | 2.62× | 3.33× | 2.87× | 1.87× | 1.26× | 6.79× | 52147 | 3.22× | 
| Kotlin/JS Gradle production | FAIL | 9 | 1.20× | 1.25× | 3.86× | 2.06× | 2.16× | 12.92× | 1.75× | 0.29× | 28817 | 1.78× | 

Ratios below 1.00× are faster or smaller than the oracle; ratios above 1.00× are slower or larger.

