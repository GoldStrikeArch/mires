// The boundary of the ReScript port: one operation per input, as libraries/diff/reference.mjs runs
// it. The input record is the JS object itself (no conversion); strings are JS strings and JSON
// documents are JSON.t values. Change objects, hunks and structured patches are records, which are
// the JS objects jsdiff returns, keys in its order; `false` for a patch that does not apply.

open Host

type options = {
  ignoreCase?: bool,
  oneChangePerToken?: bool,
  stripTrailingCr?: bool,
  ignoreWhitespace?: bool,
  newlineIsToken?: bool,
  ignoreNewlineAtEof?: bool,
  context?: int,
}

type input = {
  op: string,
  before?: JSON.t,
  after?: JSON.t,
  patch?: JSON.t,
  oldFileName?: string,
  newFileName?: string,
  oldHeader?: string,
  newHeader?: string,
  options?: options,
}

let diffOptions = input => {
  let flag = read => input.options->Option.flatMap(read) == Some(true)
  {
    Diff.ignoreCase: flag(o => o.ignoreCase),
    oneChangePerToken: flag(o => o.oneChangePerToken),
    stripTrailingCr: flag(o => o.stripTrailingCr),
    ignoreWhitespace: flag(o => o.ignoreWhitespace),
    newlineIsToken: flag(o => o.newlineIsToken),
    ignoreNewlineAtEof: flag(o => o.ignoreNewlineAtEof),
  }
}

let text = value =>
  switch value {
  | Some(JSON.String(text)) => text
  | _ => JsError.TypeError.throwWithMessage("expected a string")
  }

let document = value =>
  switch value {
  | Some(document) => document
  | None => JsError.TypeError.throwWithMessage("expected a JSON document")
  }

let elements = value =>
  switch value {
  | Some(JSON.Array(elements)) => elements
  | _ => JsError.TypeError.throwWithMessage("expected an array")
  }

let structuredPatch = input =>
  Patch.structuredPatch(
    ~oldFileName=input.oldFileName,
    ~newFileName=input.newFileName,
    text(input.before),
    text(input.after),
    ~oldHeader=input.oldHeader,
    ~newHeader=input.newHeader,
    ~context=input.options->Option.flatMap(o => o.context)->Option.getOr(4),
  )

let run = (input: input): unknown =>
  switch input.op {
  | "diffChars" => Diff.chars(text(input.before), text(input.after), diffOptions(input))->toUnknown
  | "diffWords" => Words.words(text(input.before), text(input.after), diffOptions(input))->toUnknown
  | "diffLines" => Diff.lines(text(input.before), text(input.after), diffOptions(input))->toUnknown
  | "diffJson" => Diff.json(document(input.before), document(input.after), diffOptions(input))->toUnknown
  | "diffArrays" => Diff.arrays(elements(input.before), elements(input.after), diffOptions(input))->toUnknown
  | "structuredPatch" => structuredPatch(input)->toUnknown
  | "createTwoFilesPatch" => structuredPatch(input)->Patch.formatPatch->toUnknown
  | "parsePatch" => Patch.parsePatch(text(input.patch))->toUnknown
  | "applyPatch" =>
    let patches = switch document(input.patch) {
    | String(text) => Patch.parsePatch(text)
    | Array(patches) => patches->Array.map(fromJson)
    | patch => [fromJson(patch)]
    }
    switch Patch.applyPatch(text(input.before), patches) {
    | Some(text) => toUnknown(text)
    | None => toUnknown(false)
    }
  | "reversePatch" => Patch.reversePatch(document(input.patch))
  | _ => JsError.throwWithMessage("unknown diff operation")
  }
