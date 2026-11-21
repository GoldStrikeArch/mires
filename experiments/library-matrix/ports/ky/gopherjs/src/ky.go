// Ky 2.2.0 (core/Ky.js, core/constants.js, utils/normalize.js, utils/merge.js, utils/timeout.js,
// utils/is-network-error.js and errors/*.js), the subset in libraries/ky/CONTRACT.md. Ky's async
// methods are blocking functions run on a goroutine; what Ky throws, they return as the error.

package main

import (
	"math"
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

// HTTPError, NetworkError and TimeoutError are Ky's own errors. Everything else a call can fail
// with (a TypeError, RangeError, SyntaxError or abort reason, from a host API, the transport or
// Ky's option checks) is the JS error that was thrown, a *js.Error.
type HTTPError struct {
	Response Response
	Request  Request
	// Data is the parsed or text error body, or nil when there is none.
	Data *js.Object
}

type NetworkError struct {
	Request Request
	Cause   error
}

type TimeoutError struct{ Request Request }

func (e *HTTPError) Error() string    { return "Request failed with status code " + e.Request.Method() }
func (e *NetworkError) Error() string { return "Request failed due to a network error" }
func (e *TimeoutError) Error() string { return "Request timed out" }

// errorName is error.name, as the hooks and the caller read it.
func errorName(err error) string {
	switch err := err.(type) {
	case *HTTPError:
		return "HTTPError"
	case *NetworkError:
		return "NetworkError"
	case *TimeoutError:
		return "TimeoutError"
	case *js.Error:
		return err.Get("name").String()
	}
	return "Error"
}

type Hooks struct {
	Init          []func()
	BeforeRequest []func(request Request, retryCount int)
	AfterResponse []func(request Request, response Response, retryCount int)
	BeforeRetry   []func(request Request, err error, retryCount int)
	BeforeError   []func(request Request, err error, retryCount int)
}

// RetryOptions are the caller's retry options, as the JS values given (js.Undefined where
// omitted); normalizeRetryOptions validates them.
type RetryOptions struct {
	Limit, Methods, StatusCodes, MaxRetryAfter, BackoffLimit, RetryOnTimeout *js.Object
	Delay                                                                    func(attemptCount int) float64
}

type Options struct {
	Method *string
	// Headers is a plain object of header names to values, or js.Undefined.
	Headers         *js.Object
	JSON            *js.Object
	Retry           RetryOptions
	Timeout         *js.Object
	ThrowHTTPErrors *js.Object
	Hooks           Hooks
	Fetch           func(request Request) (Response, error)
	Signal          *AbortSignal
}

// Shortcut is a body shortcut, by the media type it advertises.
type Shortcut string

const (
	JSONShortcut Shortcut = "application/json"
	TextShortcut Shortcut = "text/*"
)

type retry struct {
	limit            float64
	methods          []string
	statusCodes      []float64
	afterStatusCodes []float64
	maxRetryAfter    float64
	backoffLimit     float64
	delay            func(attemptCount int) float64
	retryOnTimeout   bool
}

var (
	// core/constants.js and utils/normalize.js
	requestMethods         = []string{"get", "post", "put", "patch", "head", "delete", "query"}
	caseInsensitiveMethods = append(append([]string{}, requestMethods...), "options", "trace")
	retryMethods           = []string{"get", "put", "head", "delete", "options", "trace", "query"}
	retryStatusCodes       = []float64{408, 413, 429, 500, 502, 503, 504}
	retryAfterStatusCodes  = []float64{413, 429, 503}
	networkErrorMessages   = []string{
		"network error",
		"NetworkError when attempting to fetch resource.",
		"The Internet connection appears to be offline.",
		"Network request failed",
		"fetch failed",
		"terminated",
		" A network error occurred.",
		"Network connection lost",
	}
)

const (
	// The maximum value of a 32bit int (see issue #117)
	maxSafeTimeout           = 2147483647.0
	maxErrorResponseBodySize = 10 * 1024 * 1024
)

func contains[T comparable](values []T, value T) bool {
	for _, candidate := range values {
		if candidate == value {
			return true
		}
	}
	return false
}

// normalizeRequestMethod is Ky's. Uses jsLowerCase so Unicode case folding matches JS's
// toLowerCase; every name in requestMethods is pure ASCII, so strings.ToUpper agrees with JS
// once matched.
func normalizeRequestMethod(input string) string {
	if contains(requestMethods, jsLowerCase(input)) {
		return strings.ToUpper(input)
	}
	return input
}

// normalizeRetryMethod: common methods for retry matching are case-insensitive, custom ones keep
// their case. Uses jsLowerCase because caseInsensitiveMethods contains "options" (Go's
// strings.ToLower folds U+0130 'İ' to ASCII 'i', whereas JS yields two code units).
func normalizeRetryMethod(method string) string {
	if lowercase := jsLowerCase(method); contains(caseInsensitiveMethods, lowercase) {
		return lowercase
	}
	return method
}

func nonNegative(value *js.Object) (float64, bool) {
	if typeTag(value) != "[object Number]" {
		return 0, false
	}
	n := value.Float()
	if math.IsNaN(n) || n < 0 {
		return 0, false
	}
	return n, true
}

// normalizeRetryOptions applies the defaults for undefined values, and Ky's checks, in Ky's order.
func normalizeRetryOptions(options RetryOptions) (retry, error) {
	normalized := retry{
		limit:            2,
		statusCodes:      retryStatusCodes,
		afterStatusCodes: retryAfterStatusCodes,
		delay:            options.Delay,
	}
	if !isUndefined(options.Limit) {
		if !isInteger(options.Limit) || options.Limit.Float() < 0 {
			return retry{}, newError("TypeError", "`retry.limit` must be a finite, non-negative integer")
		}
		normalized.limit = options.Limit.Float()
	}
	array := func(key string, value *js.Object) ([]*js.Object, error) {
		if isUndefined(value) {
			return nil, nil
		}
		if !isArray(value) {
			return nil, newError("Error", "retry."+key+" must be an array")
		}
		return elements(value), nil
	}
	methods, err := array("methods", options.Methods)
	if err != nil {
		return retry{}, err
	}
	statusCodes, err := array("statusCodes", options.StatusCodes)
	if err != nil {
		return retry{}, err
	}
	// A mistyped entry only ever fails to match, so it is reported instead.
	for _, method := range methods {
		if typeTag(method) != "[object String]" {
			return retry{}, newError("TypeError", "`retry.methods` must only contain strings")
		}
	}
	for _, status := range statusCodes {
		if !isInteger(status) {
			return retry{}, newError("TypeError", "`retry.statusCodes` must only contain numbers")
		}
	}
	methodNames := retryMethods
	if methods != nil {
		methodNames = make([]string, len(methods))
		for i, method := range methods {
			methodNames[i] = method.String()
		}
	}
	normalized.methods = make([]string, len(methodNames))
	for i, method := range methodNames {
		normalized.methods[i] = normalizeRetryMethod(method)
	}
	if statusCodes != nil {
		normalized.statusCodes = make([]float64, len(statusCodes))
		for i, status := range statusCodes {
			normalized.statusCodes[i] = status.Float()
		}
	}
	limitOption := func(key string, value *js.Object) (float64, error) {
		if isUndefined(value) {
			return math.Inf(1), nil
		}
		if limit, ok := nonNegative(value); ok {
			return limit, nil
		}
		return 0, newError("TypeError", "`retry."+key+"` must be a non-negative number or `Infinity`")
	}
	if normalized.maxRetryAfter, err = limitOption("maxRetryAfter", options.MaxRetryAfter); err != nil {
		return retry{}, err
	}
	if normalized.backoffLimit, err = limitOption("backoffLimit", options.BackoffLimit); err != nil {
		return retry{}, err
	}
	if !isUndefined(options.RetryOnTimeout) {
		if typeTag(options.RetryOnTimeout) != "[object Boolean]" {
			return retry{}, newError("TypeError", "`retry.retryOnTimeout` must be a boolean")
		}
		normalized.retryOnTimeout = options.RetryOnTimeout.Bool()
	}
	return normalized, nil
}

// validateTimeoutOption gives the milliseconds, or ok false for `false`.
func validateTimeoutOption(value *js.Object) (milliseconds float64, ok bool, err error) {
	if typeTag(value) == "[object Boolean]" && !value.Bool() {
		return 0, false, nil
	}
	milliseconds, ok = nonNegative(value)
	if !ok {
		return 0, false, newError("TypeError", "The `timeout` option must be a non-negative number or `false`")
	}
	// Infinity lands here too, since it is not a usable delay either.
	if milliseconds > maxSafeTimeout {
		return 0, false, newError("RangeError", "The `timeout` option cannot be greater than 2147483647")
	}
	return milliseconds, true, nil
}

type header struct {
	name  string
	value *js.Object
}

// lowercaseHeaders is option merging (deepMerge, mergeHeaderContainers): the headers keyed by
// their lowercased name, the last spelling of a name winning at the place of its first.
func lowercaseHeaders(headers *js.Object) []header {
	var merged []header
	positions := map[string]int{}
	for _, key := range elements(jsObject.Call("keys", headers)) {
		name, value := jsLowerCase(key.String()), headers.Get(key.String())
		if position, ok := positions[name]; ok {
			merged[position].value = value
			continue
		}
		positions[name] = len(merged)
		merged = append(merged, header{name, value})
	}
	return merged
}

// mergeHeaders sets each header on a fresh Headers, which validates it.
func mergeHeaders(headers []header) (Headers, error) {
	result := newHeaders()
	for _, header := range headers {
		if err := result.Set(header.name, header.value); err != nil {
			return Headers{}, err
		}
	}
	return result, nil
}

// isRawNetworkError is utils/is-network-error.js, inlined by Ky from is-network-error 1.3.2.
func isRawNetworkError(err error) bool {
	thrown, ok := err.(*js.Error)
	if !ok || !isError(thrown.Object) || thrown.Get("name").String() != "TypeError" {
		return false
	}
	if typeTag(thrown.Get("message")) != "[object String]" {
		return false
	}
	message := thrown.Get("message").String()
	wrapped := func(prefix string) bool { return strings.HasPrefix(message, prefix) && strings.HasSuffix(message, ")") }
	if message == "Load failed" || wrapped("Load failed (") {
		// Safari 17+ has generic message but no stack for network errors
		return isUndefined(thrown.Get("stack")) || js.Global.Get("Reflect").Call("has", thrown.Object, "__sentry_captured__").Bool()
	}
	return strings.HasPrefix(message, "error sending request for url") || message == "Failed to fetch" ||
		wrapped("Failed to fetch (") || contains(networkErrorMessages, message)
}

// isError is Ky's: an Error of this realm, or one of another realm by its internal brand.
func isError(value *js.Object) bool {
	return js.Global.Get("Error").Get("prototype").Call("isPrototypeOf", value).Bool() || typeTag(value) == "[object Error]"
}

// isJSSpace is JS whitespace: what String.prototype.trim removes and \s matches. Go's
// unicode.IsSpace adds U+0085 and lacks U+FEFF.
func isJSSpace(r rune) bool {
	switch r {
	case '\t', '\n', '\v', '\f', '\r', ' ', '\u00a0', '\u1680', '\u2028', '\u2029', '\u202f', '\u205f', '\u3000', '\ufeff':
		return true
	}
	return r >= '\u2000' && r <= '\u200a'
}

// isJSONContentType matches JSON subtypes like `json`, `problem+json` and `vnd.api+json`,
// written out rather than matched with a pattern, as Ky does. The character before `json` is
// ASCII whenever it matters, so a byte of UTF-8 decides as a code unit would.
func isJSONContentType(contentType string) bool {
	mimeType := strings.ToLower(strings.TrimFunc(strings.SplitN(contentType, ";", 2)[0], isJSSpace))
	if !strings.HasSuffix(mimeType, "json") || len(mimeType) < 5 {
		return false
	}
	switch mimeType[len(mimeType)-5] {
	case '/':
		return true
	case '.', '+', '-':
		// The separator is not the slash, so any slash in the type comes before it.
		return strings.Contains(mimeType, "/")
	}
	return false
}

// charset finds the charset parameter as /;\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))/i does:
// after the first `;` where the pattern matches, a quoted value of one or more characters, or
// else a run of characters other than `;`, `,` and whitespace. The case of `charset` is ignored
// for ASCII letters only, as a non-Unicode JS pattern ignores it.
func charset(contentType string) (string, bool) {
	rest := contentType
	for {
		index := strings.IndexByte(rest, ';')
		if index < 0 {
			return "", false
		}
		rest = rest[index+1:]
		after := strings.TrimLeftFunc(rest, isJSSpace)
		if len(after) < 7 || !equalFoldASCII(after[:7], "charset") {
			continue
		}
		after = strings.TrimLeftFunc(after[7:], isJSSpace)
		if !strings.HasPrefix(after, "=") {
			continue
		}
		value := strings.TrimLeftFunc(after[1:], isJSSpace)
		if strings.HasPrefix(value, `"`) {
			if end := strings.IndexByte(value[1:], '"'); end > 0 {
				return value[1 : end+1], true
			}
		}
		end := strings.IndexFunc(value, func(r rune) bool { return r == ';' || r == ',' || isJSSpace(r) })
		if end < 0 {
			end = len(value)
		}
		if end > 0 {
			return value[:end], true
		}
	}
}

func equalFoldASCII(text, lowercase string) bool {
	for i := 0; i < len(text); i++ {
		c := text[i]
		if c >= 'A' && c <= 'Z' {
			c += 'a' - 'A'
		}
		if c != lowercase[i] {
			return false
		}
	}
	return true
}

// createTextDecoder gives the charset's decoder, or UTF-8 for none or an unknown label.
func createTextDecoder(contentType string) TextDecoder {
	if label, ok := charset(contentType); ok {
		if decoder, err := newTextDecoder(label); err == nil {
			return decoder
		}
	}
	return utf8Decoder()
}

// readResponseText reads an error body as text, or ok false when it cannot be read or is larger
// than maxErrorResponseBodySize. JSON uses UTF-8 regardless of the charset parameter (RFC 8259).
func readResponseText(response Response) (string, bool) {
	if !response.HasBody() {
		text, err := response.Text()
		return text, err == nil
	}
	contentType, _ := response.Headers().Get("content-type")
	decoder := utf8Decoder()
	if !isJSONContentType(contentType) {
		decoder = createTextDecoder(contentType)
	}
	bytes, length, err := response.Bytes()
	if err != nil || length > maxErrorResponseBodySize {
		return "", false
	}
	return decoder.Decode(bytes), true
}

// getResponseData is #getResponseData: the parsed JSON or the text of an error body, or nil when
// it is empty, unreadable or unparsable.
func getResponseData(response Response) *js.Object {
	text, ok := readResponseText(response)
	if !ok || text == "" {
		return nil
	}
	if contentType, _ := response.Headers().Get("content-type"); !isJSONContentType(contentType) {
		return js.Global.Call("String", text)
	}
	var data *js.Object
	if catch(func() { data = js.Global.Get("JSON").Call("parse", text) }) != nil {
		return nil
	}
	return data
}

type Ky struct {
	request         Request
	abortController AbortController
	// signal is #options.signal: the user's signal and the abort controller's, combined.
	signal                  AbortSignal
	retryCount              int
	originalRequest         *Request
	userProvidedAbortSignal *AbortSignal
	retry                   retry
	timeout                 *float64
	throwHTTPErrors         bool
	hooks                   Hooks
	// transport is the `fetch` option.
	transport func(request Request) (Response, error)
}

func createManagedSignal(user *AbortSignal, controller AbortController) AbortSignal {
	if user == nil {
		return controller.Signal()
	}
	return anySignal(*user, controller.Signal())
}

func newKy(input string, options Options) (*Ky, error) {
	var headers []header
	if !isUndefined(options.Headers) {
		headers = lowercaseHeaders(options.Headers)
	}
	requestHeaders, err := mergeHeaders(headers)
	if err != nil {
		return nil, err
	}
	method := "GET"
	if options.Method != nil {
		method = *options.Method
	}
	method = normalizeRequestMethod(method)
	retry, err := normalizeRetryOptions(options.Retry)
	if err != nil {
		return nil, err
	}
	// Defaults only replace undefined, so a null reaches the validation and is reported.
	throwHTTPErrors := true
	if !isUndefined(options.ThrowHTTPErrors) {
		if typeTag(options.ThrowHTTPErrors) != "[object Boolean]" {
			return nil, newError("TypeError", "The `throwHttpErrors` option must be a boolean or a function")
		}
		throwHTTPErrors = options.ThrowHTTPErrors.Bool()
	}
	timeoutValue := options.Timeout
	if isUndefined(timeoutValue) {
		timeoutValue = js.Global.Call("Number", 10000)
	}
	milliseconds, hasTimeout, err := validateTimeoutOption(timeoutValue)
	if err != nil {
		return nil, err
	}
	var timeout *float64
	if hasTimeout {
		timeout = &milliseconds
	}
	abortController := newAbortController()
	signal := createManagedSignal(options.Signal, abortController)
	// A content-type from options.headers counts as user-provided.
	userProvidedContentType := false
	for _, header := range headers {
		userProvidedContentType = userProvidedContentType || header.name == "content-type"
	}
	var body *string
	if !isUndefined(options.JSON) {
		text := js.Global.Get("JSON").Call("stringify", options.JSON).String()
		body = &text
		if !userProvidedContentType {
			if err := requestHeaders.Set("content-type", "application/json"); err != nil {
				return nil, err
			}
		}
	}
	request, err := newRequest(input, method, requestHeaders, body, signal)
	if err != nil {
		return nil, err
	}
	return &Ky{
		request:                 request,
		abortController:         abortController,
		signal:                  signal,
		userProvidedAbortSignal: options.Signal,
		retry:                   retry,
		timeout:                 timeout,
		throwHTTPErrors:         throwHTTPErrors,
		hooks:                   options.Hooks,
		transport:               options.Fetch,
	}, nil
}

// send is ky(input, options)[shortcut](): the init hooks, the request, the run and the body read.
// A construction error fails without running the beforeError hooks, and so does a body that is
// not JSON.
func send(input string, options Options, shortcut Shortcut) (*js.Object, error) {
	for _, hook := range options.Hooks.Init {
		hook()
	}
	ky, err := newKy(input, options)
	if err != nil {
		return nil, err
	}
	// Before dispatch, ky.request is the request that will be sent.
	headers := ky.request.Headers()
	if accept, _ := headers.Get("accept"); accept != "" {
		headers.Set("accept", accept)
	} else {
		headers.Set("accept", string(shortcut))
	}
	response, err := ky.run()
	if err != nil {
		return nil, ky.processedError(err)
	}
	text, err := response.Text()
	if err != nil {
		return nil, err
	}
	if shortcut == TextShortcut {
		return js.Global.Call("String", text), nil
	}
	var result *js.Object
	err = catch(func() { result = js.Global.Get("JSON").Call("parse", text) })
	return result, err
}

// sentRequest is #sentRequest: fetch replaces the request with the clone it prepares for a
// possible retry, so the request that was actually sent is reported.
func (k *Ky) sentRequest() Request {
	if k.originalRequest != nil {
		return *k.originalRequest
	}
	return k.request
}

// throwIfAbortedByUser: Chromium reports a user abort during a fetch as a network error; the
// abort reason is surfaced instead.
func (k *Ky) throwIfAbortedByUser() error {
	if k.userProvidedAbortSignal == nil {
		return nil
	}
	return k.userProvidedAbortSignal.ThrowIfAborted()
}

func (k *Ky) run() (Response, error) {
	// A hook runs inside #raceWithTotalTimeout(), which first throws a user abort.
	for _, hook := range k.hooks.BeforeRequest {
		if err := k.throwIfAbortedByUser(); err != nil {
			return Response{}, err
		}
		hook(k.request, 0)
	}
	response, err := k.retryRequest(nil)
	for err == nil {
		for _, hook := range k.hooks.AfterResponse {
			if err := k.throwIfAbortedByUser(); err != nil {
				return Response{}, err
			}
			hook(k.request, response, k.retryCount)
		}
		if response.OK() || !k.throwHTTPErrors {
			return response, nil
		}
		httpError := &HTTPError{Response: response, Request: k.sentRequest(), Data: getResponseData(response)}
		if err := k.throwIfAbortedByUser(); err != nil {
			return Response{}, err
		}
		response, err = k.retryRequest(httpError)
	}
	return Response{}, err
}

// retryRequest is #retry() and #retryFromError(), which call each other in Ky, as one loop: each
// pending error either is retried (the wait, the beforeRetry hooks and the next fetch) or ends it.
func (k *Ky) retryRequest(pending error) (Response, error) {
	for {
		if pending != nil {
			if err := k.prepareRetry(pending); err != nil {
				return Response{}, err
			}
		}
		response, err := k.fetch()
		if err == nil {
			return response, nil
		}
		pending = err
	}
}

// prepareRetry is #retryFromError() up to its final #retry().
func (k *Ky) prepareRetry(cause error) error {
	if err := k.throwIfAbortedByUser(); err != nil {
		return err
	}
	delay, err := k.calculateRetryDelay(cause)
	if err != nil {
		return err
	}
	sleep(math.Min(delay, maxSafeTimeout))
	// Reset the abort controller if it was aborted (on a timeout retry), so hooks and the retried
	// request get a fresh signal.
	if k.abortController.Signal().Aborted() {
		k.abortController = newAbortController()
		k.signal = createManagedSignal(k.userProvidedAbortSignal, k.abortController)
		if k.request, err = k.request.WithSignal(k.signal); err != nil {
			return err
		}
	}
	for _, hook := range k.hooks.BeforeRetry {
		if err := k.throwIfAbortedByUser(); err != nil {
			return err
		}
		hook(k.request, cause, k.retryCount+1)
	}
	k.retryCount++
	return nil
}

func (k *Ky) fetch() (Response, error) {
	// Cloning is done here to prepare in advance for retries, and skipped when they are disabled.
	var retryRequest *Request
	if k.retry.limit > 0 {
		clone, err := k.request.Clone()
		if err == nil {
			clone, err = clone.WithSignal(k.signal)
		}
		if err != nil {
			return Response{}, err
		}
		retryRequest = &clone
	}
	request := k.request
	k.originalRequest = &request
	if retryRequest != nil {
		k.request = *retryRequest
	}
	var response Response
	var err error
	if k.timeout == nil {
		response, err = k.transport(request)
	} else {
		response, err = k.race(request, *k.timeout)
	}
	if err == nil {
		return response, nil
	}
	if abort := k.throwIfAbortedByUser(); abort != nil {
		return Response{}, abort
	}
	if isRawNetworkError(err) {
		return Response{}, &NetworkError{Request: request, Cause: err}
	}
	return Response{}, err
}

// race is utils/timeout.js: the transport raced against a timer that aborts the controller and
// fails with a TimeoutError. The first of the two to finish decides, as the first settlement of
// Ky's promise does; a response arriving after the timeout is discarded.
func (k *Ky) race(request Request, delay float64) (Response, error) {
	type outcome struct {
		response Response
		err      error
	}
	decided := make(chan outcome, 1)
	finished := false
	decide := func(result outcome) {
		if !finished {
			finished = true
			decided <- result
		}
	}
	timer := setTimeout(func() {
		if !finished {
			// Decided before the abort: a goroutine the abort wakes may run at once.
			decide(outcome{err: &TimeoutError{Request: request}})
			k.abortController.Abort()
		}
	}, delay)
	go func() {
		response, err := k.transport(request)
		decide(outcome{response, err})
	}()
	result := <-decided
	clearTimeout(timer)
	return result.response, result.err
}

// calculateDelay is #calculateDelay(): the retry.delay backoff, capped by retry.backoffLimit.
func (k *Ky) calculateDelay() (float64, error) {
	retryDelay := k.retry.delay(k.retryCount + 1)
	if !(retryDelay >= 0) {
		return 0, newError("TypeError", "`retry.delay` must return a non-negative number or `Infinity`")
	}
	return math.Min(k.retry.backoffLimit, retryDelay), nil
}

// calculateRetryDelay is #calculateRetryDelay(): the wait before retrying err, or err itself when
// it is not retried.
func (k *Ky) calculateRetryDelay(err error) (float64, error) {
	if float64(k.retryCount) >= k.retry.limit {
		return 0, err
	}
	if !contains(k.retry.methods, normalizeRetryMethod(k.request.Method())) {
		return 0, err
	}
	switch cause := err.(type) {
	case *TimeoutError:
		if !k.retry.retryOnTimeout {
			return 0, err
		}
		return k.calculateDelay()
	case *HTTPError:
		status := float64(cause.Response.Status())
		if !contains(k.retry.statusCodes, status) {
			return 0, err
		}
		if header, ok := getRetryTimingHeader(cause.Response.Headers()); ok && contains(k.retry.afterStatusCodes, status) {
			// Malformed retry timing headers should not disable retries; they only lose their
			// server-provided timing. No jitter applies to a server's timing.
			if after, ok := calculateRetryTimingDelay(header); ok {
				return math.Min(k.retry.maxRetryAfter, after), nil
			}
			return k.calculateDelay()
		}
		if status == 413 {
			return 0, err
		}
		return k.calculateDelay()
	case *NetworkError:
		return k.calculateDelay()
	}
	// Only known retriable error types are retried.
	return 0, err
}

// processedError is #throwProcessedError(): the beforeError hooks see the error, which then
// fails the call.
func (k *Ky) processedError(err error) error {
	for _, hook := range k.hooks.BeforeError {
		hook(k.sentRequest(), err, k.retryCount)
	}
	return err
}
