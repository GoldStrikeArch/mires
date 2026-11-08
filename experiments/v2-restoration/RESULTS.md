# v2 restoration results

Append-only log written by `measure.mjs`. Each block is one run; receipts live in
`build/v2-restoration/<label>/`. Never edit or delete a recorded block. See README.md for the protocol.

## R0 — 2026-11-08T08:18:02.610Z

- HEAD `0111435dbebe69542f1be1a8f1a325aab7bd572f`, 2 dirty path(s); compiler source digest `93c7576b18c7` (differs from docs/v2-source-identity.json)
- jsc-link `7e74778258f9`, codegen dylib `6bb0690ff552`, node v26.8.1, built: yes
- receipts: `build/v2-restoration/R0/`

| target | file | raw | min | gzip | brotli | status |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| compact | fp.esm.min.js | 680202 | 515481 | 110636 | 80491 | ok |
| compact | fp.min.js | 685034 | 515459 | 110398 | 80412 | ok |
| compact | fp.umd.min.js | 685305 | 515654 | 110483 | 80529 | ok |
| compact | raw/program.js | 668404 | — | 108757 | 83969 | ok |
| idiomatic | fp.esm.min.js | 1069248 | 822496 | 186452 | 135211 | ok |
| idiomatic | fp.min.js | 1075878 | 822477 | 186256 | 135123 | ok |
| idiomatic | fp.umd.min.js | 1076149 | 822672 | 186330 | 135140 | ok |
| idiomatic | raw/program.js | 1056209 | — | 196228 | 144982 | ok |
| corpus | minisearch/index.min.js | 342881 | 252772 | 57993 | 45553 | ok |
| corpus | minisearch/raw/program.js | 321386 | — | 58133 | 46585 | ok |
| corpus | marked/index.min.js | 135418 | 100807 | 25813 | 21086 | ok |
| corpus | marked/raw/program.js | 119462 | — | 24440 | 20049 | ok |
| corpus | fflate/index.min.js | 79519 | 57149 | 14083 | 12168 | ok |
| corpus | fflate/raw/program.js | 63941 | — | 12111 | 10453 | ok |
| corpus | idb/index.min.js | 119669 | 86723 | 21456 | 18074 | ok |
| corpus | idb/raw/program.js | 100514 | — | 19588 | 16505 | ok |
| corpus | idb_lines/index.min.js | 128973 | 94678 | 23516 | 19612 | ok |
| corpus | idb_lines/raw/program.js | 110001 | — | 21890 | 18219 | ok |
| corpus | ky/index.min.js | 226962 | 169462 | 41606 | 33527 | ok |
| corpus | ky/raw/program.js | 202095 | — | 40362 | 32718 | ok |
| corpus | uplot/index.min.js | 14179 | 10054 | 3406 | 2729 | ok |
| corpus | uplot/raw/program.js | 13634 | — | 3417 | 2762 | ok |
| corpus | explorer/index.min.js | 474712 | 354830 | 82943 | 64133 | ok |
| corpus | explorer/raw/program.js | 453367 | — | 84046 | 66713 | ok |
| corpus | workbox/index.min.js | 95304 | 68851 | 17479 | 14663 | ok |
| corpus | workbox/raw/program.js | 78440 | — | 15724 | 12942 | ok |
| corpus | workbox_direct/index.min.js | 16158 | 12286 | 3623 | 3112 | ok |
| corpus | workbox_direct/raw/program.js | 12885 | — | 2863 | 2442 | ok |

| target | wall s | peak RSS MiB | status |
| --- | ---: | ---: | --- |
| compact | 88.4 | 2706 | ok |
| idiomatic | 327.1 | 2876 | ok |
| corpus | 156.8 | 2823 | ok |

| workload | p50 ms | p95 ms | calls/sample | samples |
| --- | ---: | ---: | ---: | ---: |
| marked | 399.75 | 419.66 | 5 | 9 |
| minisearch | 4.78 | 7.43 | 4 | 9 |
| fflate | 16.27 | 25.26 | 3 | 9 |

- corpus pure run: passed; compact kernels: 58/58; idiomatic kernels: 58/58; corpus artifacts vs build.json: identical
- pro: ok, wall 654.1 s, peak RSS 2856 MiB

## S2-compile-cost — 2026-11-08T11:23:00.159Z

- HEAD `410e1543fc3fa238b1c9a671ce8f28ed0914ace3`, 0 dirty path(s); compiler source digest `ef97ab4b7c15` (differs from docs/v2-source-identity.json)
- jsc-link `a307003bff23`, codegen dylib `2667f2f5bec3`, node v26.8.1, built: yes
- receipts: `build/v2-restoration/S2-compile-cost/`

| target | file | raw | min | gzip | brotli | status |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| compact | fp.esm.min.js | 680202 | 515481 | 110636 | 80491 | ok |
| compact | fp.min.js | 685034 | 515459 | 110398 | 80412 | ok |
| compact | fp.umd.min.js | 685305 | 515654 | 110483 | 80529 | ok |
| compact | raw/program.js | 668404 | — | 108757 | 83969 | ok |

| target | wall s | peak RSS MiB | status |
| --- | ---: | ---: | --- |
| compact | 86.1 | 2888 | ok |

- compact kernels: 58/58
- pro: ok, wall 205.5 s, peak RSS 3082 MiB
