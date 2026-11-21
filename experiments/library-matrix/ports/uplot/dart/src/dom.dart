// The browser APIs the chart uses, as dart:js_interop extension types: the document, the canvas
// and its 2D context, Path2D, devicePixelRatio and JS Math.round.
import 'dart:js_interop';

@JS()
external Document get document;

/// window.devicePixelRatio, which uPlot reads as its pxRatio.
@JS()
external double get devicePixelRatio;

/// JS Math.round: halves round toward +Infinity (Dart's round() rounds them away from zero).
@JS('Math.round')
external double jsRound(double x);

/// JS Math.log10: exact host base-10 logarithm for uPlot's rangeNum.
@JS('Math.log10')
external double jsLog10(double x);

extension type Document._(JSObject _) implements JSObject {
  external Element get body;
  external Canvas createElement(String tagName);
}

extension type Element._(JSObject _) implements JSObject {
  external void appendChild(Canvas node);
}

extension type Canvas._(JSObject _) implements JSObject {
  external double get width;
  external set width(double value);
  external double get height;
  external set height(double value);
  external Context2D getContext(String contextId);
  external void remove();
}

extension type Context2D._(JSObject _) implements JSObject {
  external set strokeStyle(String value);
  external set lineWidth(double value);
  external set lineJoin(String value);
  external set lineCap(String value);
  external void save();
  external void restore();
  external void translate(double x, double y);
  external void clip(Path2D path);
  external void stroke(Path2D path);
}

extension type Path2D._(JSObject _) implements JSObject {
  external Path2D();
  external void lineTo(double x, double y);
  external void rect(double x, double y, double w, double h);
}
