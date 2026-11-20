// The boundary of the Dart port: one operation per input, as libraries/diff/reference.mjs runs it.
// The input is the JS object itself, seen through dart:js_interop extension types (no copy).
// Strings are JS strings (Dart strings on the web, UTF-16); JSON documents and array elements stay
// JS values. Results are plain JS objects (JSObject) and arrays (List.toJS) in jsdiff's key order,
// `false` for a patch that does not apply. main() publishes run on globalThis.__matrixExports for
// the generated facade.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'diff.dart';
import 'patch.dart';
import 'words.dart';

extension type Input._(JSObject _) implements JSObject {
  external String get op;
  external JSAny? get before;
  external JSAny? get after;
  external JSAny? get patch;
  external JSAny? get oldFileName;
  external JSAny? get newFileName;
  external JSAny? get oldHeader;
  external JSAny? get newHeader;
  external OptionsJson? get options;
}

extension type OptionsJson._(JSObject _) implements JSObject {
  external bool? get ignoreCase;
  external bool? get oneChangePerToken;
  external bool? get stripTrailingCr;
  external bool? get ignoreWhitespace;
  external bool? get newlineIsToken;
  external bool? get ignoreNewlineAtEof;
  external int? get context;
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}

DiffOptions _options(OptionsJson? json) => DiffOptions(
      ignoreCase: json?.ignoreCase == true,
      oneChangePerToken: json?.oneChangePerToken == true,
      stripTrailingCr: json?.stripTrailingCr == true,
      ignoreWhitespace: json?.ignoreWhitespace == true,
      newlineIsToken: json?.newlineIsToken == true,
      ignoreNewlineAtEof: json?.ignoreNewlineAtEof == true,
    );

JSArray<JSObject> _changes<V extends JSAny?>(List<Change<V>> changes) => [
      for (final change in changes)
        JSObject()
          ..['count'] = change.count.toJS
          ..['added'] = change.added.toJS
          ..['removed'] = change.removed.toJS
          ..['value'] = change.value,
    ].toJS;

JSArray<JSObject> _textDiff(TextDiff diff, Input input) => _changes([
      for (final change in diff.diff(input.before, input.after, _options(input.options)))
        Change(change.count, change.added, change.removed, change.value.toJS),
    ]);

FilePatch _patch(Input input) => structuredPatch(
      input.oldFileName,
      input.newFileName,
      (input.before as JSString).toDart,
      (input.after as JSString).toDart,
      input.oldHeader,
      input.newHeader,
      input.options?.context ?? 4,
    );

JSAny? run(Input input) => switch (input.op) {
      'diffChars' => _textDiff(CharacterDiff(), input),
      'diffWords' => _textDiff(WordDiff(), input),
      'diffLines' => _textDiff(LineDiff(), input),
      'diffJson' => _textDiff(JsonDiff(), input),
      'diffArrays' => _changes(ArrayDiff().diffTokens(
          (input.before as JSArray<JSAny?>).toDart, (input.after as JSArray<JSAny?>).toDart, _options(input.options))),
      'structuredPatch' => _patch(input).toJSObject(),
      'createTwoFilesPatch' => formatPatch(_patch(input)).toJS,
      'parsePatch' => [for (final patch in parsePatch((input.patch as JSString).toDart)) patch.toJSObject()].toJS,
      'applyPatch' => _applyPatch(input),
      'reversePatch' => reversePatch(input.patch!),
      _ => throw ArgumentError('unknown diff operation'),
    };

JSAny _applyPatch(Input input) {
  final patch = input.patch;
  final patches = patch.typeofEquals('string')
      ? parsePatch((patch as JSString).toDart)
      : patch.isA<JSArray>()
          ? [for (final single in (patch as JSArray<JSAny?>).toDart) readFilePatch(single)]
          : [readFilePatch(patch)];
  return applyPatch((input.before as JSString).toDart, patches)?.toJS ?? false.toJS;
}
