library;

import 'dart:js_interop';

import 'browser.dart';
import 'dom.dart';
import 'fp_async.dart';
import 'host.dart';

const defaultText = 'mmMwWLliI0fiflO&1';

const presetOrder = [
  'default',
  'apple',
  'serif',
  'sans',
  'mono',
  'min',
  'system',
];

JSAny? getFontPreferences([JSAny? _]) {
  return withNaturalFonts((document, container, iframeWindow) {
    final elements = <String, JSAny?>{};
    final sizes = jsObject();
    for (final key in presetOrder) {
      final element = method(document, 'createElement', ['span'.toJS]);
      setp(element, 'textContent', _presetText(key).toJS);
      setp(getp(element, 'style'), 'whiteSpace', 'nowrap'.toJS);
      _applyPresetStyle(element, key);
      elements[key] = element;
      method(container, 'append', [
        method(document, 'createElement', ['br'.toJS]),
        element,
      ]);
    }
    final shouldNormalize = isChromium() && isChromium128OrNewer();
    for (final key in presetOrder) {
      final rawWidth = getp(method(elements[key], 'getBoundingClientRect'), 'width');
      if (shouldNormalize) {
        setp(
          sizes,
          key,
          fromDart(normalizeFontMeasurement(
            jsNumber(rawWidth) * jsNumber(getp(iframeWindow, 'devicePixelRatio')),
          )),
        );
      } else {
        setp(sizes, key, rawWidth);
      }
    }
    return sizes;
  });
}

String _presetText(String key) => defaultText;

void _applyPresetStyle(JSAny? element, String key) {
  final style = getp(element, 'style');
  switch (key) {
    case 'apple':
      setp(style, 'font', '-apple-system-body'.toJS);
    case 'serif':
      setp(style, 'fontFamily', 'serif'.toJS);
    case 'sans':
      setp(style, 'fontFamily', 'sans-serif'.toJS);
    case 'mono':
      setp(style, 'fontFamily', 'monospace'.toJS);
    case 'min':
      setp(style, 'fontSize', '1px'.toJS);
    case 'system':
      setp(style, 'fontFamily', 'system-ui'.toJS);
  }
}

double normalizeFontMeasurement(double value) {
  final decimals = isAndroid() ? 0 : 3;
  final pow = mathPow(10, decimals);
  return mathFloor(value * pow) / pow;
}

JSAny? withNaturalFonts(
  JSAny? Function(JSAny? document, JSAny? container, JSAny? iframeWindow) action, [
  int containerWidthPx = 4000,
]) {
  return futureToPromise(withIframe(
    ((JSAny? iframe, JSAny? iframeWindow) {
      final iframeDocument = getp(iframeWindow, 'document');
      final iframeBody = getp(iframeDocument, 'body');
      final bodyStyle = getp(iframeBody, 'style');
      setp(bodyStyle, 'width', '${containerWidthPx}px'.toJS);
      setp(bodyStyle, 'webkitTextSizeAdjust', 'none'.toJS);
      setp(bodyStyle, 'textSizeAdjust', 'none'.toJS);
      if (isChromium()) {
        setp(
          getp(iframeBody, 'style'),
          'zoom',
          fromDart('${1 / jsNumber(getp(iframeWindow, 'devicePixelRatio'))}'),
        );
      } else if (isWebKit()) {
        setp(getp(iframeBody, 'style'), 'zoom', 'reset'.toJS);
      }
      final linesOfText = method(iframeDocument, 'createElement', ['div'.toJS]);
      final words = List.filled(containerWidthPx ~/ 20, 'word').join(' ');
      setp(linesOfText, 'textContent', words.toJS);
      method(iframeBody, 'appendChild', [linesOfText]);
      return action(iframeDocument, iframeBody, iframeWindow);
    }).toJS,
    '<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1">',
  ));
}
