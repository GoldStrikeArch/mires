# Workbox 7.4.1

The reference uses the published production builds of workbox-core, workbox-routing, workbox-precaching and workbox-strategies. The port implements revisioned precache keys, duplicate and conflicting manifest entries, install and activation reports, precache route URL variations, GET route selection, and CacheFirst, CacheOnly and NetworkOnly responses. Precache routes take precedence over runtime routes.

Each scenario allocates dedicated precache and runtime cache names and deletes both on exit. Install reports distinguish newly cached and previously cached URLs. Activation reports the sorted URLs removed from the precache. A fetch returns the HTTP status and full response text, null for an unmatched route, or an array containing `no-response` for a failed strategy. Two static resources served from the test origin provide the network responses. The offline fixture rejects a network attempt. Remote hosts are never part of the workload.

Fetch steps that supply a path and method construct a native Request before routing. Fetch events use the Request supplied by the browser.

The matrix times operations on real browser CacheStorage, Request, Response and same-origin Fetch. The page driver supplies an event lifetime for Workbox's public methods. The service-worker integration uses the same exported operations during native install, activate, fetch and message events. Each benchmark runs installation, activation and 24 fetches, and validates every response.

Run `node verify-worker.mjs chrome` or `node verify-worker.mjs firefox` here after building the ports. Optional language arguments select individual artifacts, including `reference`. The verifier links the chosen production module into a classic worker for both browsers. A controlled page checks installation and activation reports, sends messages through a MessageChannel, then checks precached content and runtime cache hits after the network fixture goes offline.
