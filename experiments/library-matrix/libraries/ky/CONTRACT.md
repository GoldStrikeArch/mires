# Ky 2.2.0

Each call runs a list of independent Fetch requests against a deterministic injected transport. Every request and response is a native Fetch object. The configured transport records the method, absolute URL, selected headers and consumed request body. It returns a real Response, raises a network TypeError or waits for the real request signal to abort. The matrix makes no network requests.

A request supplies an absolute `url`, HTTP `method`, `headers`, optional JSON body `json`, body shortcut `read` (json or text), a list of transport `replies`, retry `limit`, `backoffLimit`, `maxRetryAfter`, `retryOnTimeout`, `throwHttpErrors`, a per-attempt millisecond `timeout` (or false) and `abort` (none, before dispatch, at fetch or during the beforeRetry hook). Optional `methods` and `statusCodes` override Ky's retry lists. HTTP replies supply a status, string or null body and headers. The last reply repeats if retries exhaust the list.

Construction errors produce a result for the affected request and the batch continues. A JSON body supplies a default Content-Type only when the header is absent. Retry matching ignores case for common HTTP methods and preserves the case of custom methods.

The contract records init, beforeRequest, afterResponse, beforeRetry and beforeError hooks in order, the one-based argument and delay returned by the exponential backoff callback, and each dispatch. Hooks set the initial and retry headers on the request. Numeric Retry-After headers override exponential backoff on eligible statuses, with the configured cap. The observable result includes either the parsed JSON or text, or the error name, HTTP status and parsed HTTP error data. Errors outside the HTTP path have null status and data. The benchmark issues 24 JSON requests, including POST bodies, through the same fixture and hook chain.

The pinned package's original traces and results are in `expected.json`. Use `node verify.mjs rust` in this directory to check any built port. Regenerate the fixture with `node verify.mjs reference --capture`. Request bodies are observed through the standard `Request.text()` API, which gives the empty string when no payload is present.
