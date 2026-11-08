library;

import 'dart:js_interop';

import 'browser.dart';
import 'host.dart';

const imageUnsupported = 'unsupported';
const imageSkipped = 'skipped';
const imageUnstable = 'unstable';

JSAny? getCanvasFingerprint([JSAny? _]) {
  return getUnstableCanvasFingerprint(_doesBrowserPerformAntiFingerprinting().toJS);
}

JSAny? getUnstableCanvasFingerprint([JSAny? skipImages]) {
  var winding = false;
  late JSAny? geometry;
  late JSAny? text;
  final pair = _makeCanvasContext();
  final canvas = pair.$1;
  final context = pair.$2;
  if (!_isSupported(canvas, context)) {
    geometry = text = imageUnsupported.toJS;
  } else {
    winding = _doesSupportWinding(context);
    if (jsTruthy(skipImages)) {
      geometry = text = imageSkipped.toJS;
    } else {
      final images = _renderImages(canvas, context);
      geometry = images.$1;
      text = images.$2;
    }
  }
  final result = jsObject();
  setp(result, 'winding', winding.toJS);
  setp(result, 'geometry', geometry);
  setp(result, 'text', text);
  return result;
}

(JSAny?, JSAny?) _makeCanvasContext() {
  final canvas = method(jsDocument, 'createElement', ['canvas'.toJS]);
  setp(canvas, 'width', 1.toJS);
  setp(canvas, 'height', 1.toJS);
  return (canvas, method(canvas, 'getContext', ['2d'.toJS]));
}

bool _isSupported(JSAny? canvas, JSAny? context) {
  return jsTruthy(context) && jsTruthy(getp(canvas, 'toDataURL'));
}

bool _doesSupportWinding(JSAny? context) {
  method(context, 'rect', [0.toJS, 0.toJS, 10.toJS, 10.toJS]);
  method(context, 'rect', [2.toJS, 2.toJS, 6.toJS, 6.toJS]);
  return !jsTruthy(method(context, 'isPointInPath', [5.toJS, 5.toJS, 'evenodd'.toJS]));
}

(JSAny?, JSAny?) _renderImages(JSAny? canvas, JSAny? context) {
  _renderTextImage(canvas, context);
  final textImage1 = method(canvas, 'toDataURL');
  final textImage2 = method(canvas, 'toDataURL');
  if (jsString(textImage1) != jsString(textImage2)) {
    return (imageUnstable.toJS, imageUnstable.toJS);
  }
  _renderGeometryImage(canvas, context);
  return (method(canvas, 'toDataURL'), textImage1);
}

void _renderTextImage(JSAny? canvas, JSAny? context) {
  setp(canvas, 'width', 240.toJS);
  setp(canvas, 'height', 60.toJS);
  setp(context, 'textBaseline', 'alphabetic'.toJS);
  setp(context, 'fillStyle', '#f60'.toJS);
  method(context, 'fillRect', [100.toJS, 1.toJS, 62.toJS, 20.toJS]);
  setp(context, 'fillStyle', '#069'.toJS);
  setp(context, 'font', '11pt "Times New Roman"'.toJS);
  final printedText = 'Cwm fjordbank gly ${fromCharCode2(55357, 56835)}';
  method(context, 'fillText', [printedText.toJS, 2.toJS, 15.toJS]);
  setp(context, 'fillStyle', 'rgba(102, 204, 0, 0.2)'.toJS);
  setp(context, 'font', '18pt Arial'.toJS);
  method(context, 'fillText', [printedText.toJS, 4.toJS, 45.toJS]);
}

void _renderGeometryImage(JSAny? canvas, JSAny? context) {
  setp(canvas, 'width', 122.toJS);
  setp(canvas, 'height', 110.toJS);
  setp(context, 'globalCompositeOperation', 'multiply'.toJS);
  const circles = [
    ('#f2f', 40, 40),
    ('#2ff', 80, 40),
    ('#ff2', 60, 80),
  ];
  for (final (color, x, y) in circles) {
    setp(context, 'fillStyle', color.toJS);
    method(context, 'beginPath');
    method(context, 'arc', [x.toJS, y.toJS, 40.toJS, 0.toJS, (mathPI() * 2).toJS, true.toJS]);
    method(context, 'closePath');
    method(context, 'fill');
  }
  setp(context, 'fillStyle', '#f9c'.toJS);
  method(context, 'arc', [60.toJS, 60.toJS, 60.toJS, 0.toJS, (mathPI() * 2).toJS, true.toJS]);
  method(context, 'arc', [60.toJS, 60.toJS, 20.toJS, 0.toJS, (mathPI() * 2).toJS, true.toJS]);
  method(context, 'fill', ['evenodd'.toJS]);
}

bool _doesBrowserPerformAntiFingerprinting() {
  final isSafari17OrAbove = isWebKit() && isWebKit616OrNewer() && isSafariWebKit();
  final isFirefox120OrAbove = isGecko() && isGecko120OrNewer();
  return isSafari17OrAbove || isFirefox120OrAbove;
}
