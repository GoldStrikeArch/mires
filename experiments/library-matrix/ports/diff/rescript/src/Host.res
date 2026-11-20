// The host calls the port makes beyond ReScript's own JavaScript bindings: TextEncoder and
// TextDecoder, which jsdiff itself uses for quoted names; Boolean, for JavaScript truthiness;
// String.prototype.match with a global RegExp; and identity casts between the JSON input and
// the records that have its shape (a ReScript record is the JS object).

external toUnknown: 'a => unknown = "%identity"
external fromUnknown: unknown => 'a = "%identity"
/** The JSON input as the record type that has its shape. */
external fromJson: JSON.t => 'a = "%identity"

@val external truthy: 'a => bool = "Boolean"

/** `string.match(regexp)` for a global RegExp: every match, or null. */
@send external matchAll: (string, RegExp.t) => Null.t<array<string>> = "match"

type textEncoder
@new external textEncoder: unit => textEncoder = "TextEncoder"
/** UTF-8, with U+FFFD for an isolated surrogate. */
@send external encode: (textEncoder, string) => Uint8Array.t = "encode"

type textDecoder
@new external textDecoder: unit => textDecoder = "TextDecoder"
/** UTF-8 with each maximal ill-formed subpart replaced by U+FFFD, and a leading BOM consumed. */
@send external decode: (textDecoder, Uint8Array.t) => string = "decode"
