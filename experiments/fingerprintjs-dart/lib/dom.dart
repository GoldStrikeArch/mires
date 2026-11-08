library;

import 'dart:js_interop';

import 'data.dart';
import 'fp_async.dart';
import 'host.dart';

JSAny? withIframeExport(JSAny? action, [JSAny? initialHtml, JSAny? poll]) {
  final html = initialHtml.isUndefinedOrNull ? null : jsString(initialHtml);
  final interval = poll.isUndefinedOrNull ? 50.0 : jsNumber(poll);
  return withIframeBridge(
    futureToPromise(_setupIframe(html, interval)),
    action,
  );
}

Future<JSAny?> withIframe(
  JSAny? action, [
  String? initialHtml,
  double domPollInterval = 50,
]) async {
  return _awaitJs(withIframeExport(
    action,
    initialHtml == null ? jsUndefined : initialHtml.toJS,
    domPollInterval.toJS,
  ));
}

Future<JSAny?> _setupIframe(String? initialHtml, double domPollInterval) async {
  final d = jsDocument;
  while (!jsTruthy(getp(d, 'body'))) {
    await _awaitJs(wait(domPollInterval));
  }
  final iframe = method(d, 'createElement', ['iframe'.toJS]);
  await _waitForIframe(d, iframe, initialHtml);
  while (!jsTruthy(
    getp(getp(getp(iframe, 'contentWindow'), 'document'), 'body'),
  )) {
    await _awaitJs(wait(domPollInterval));
  }
  final pair = jsObject();
  setp(pair, 'iframe', iframe);
  setp(pair, 'window', getp(iframe, 'contentWindow'));
  return pair;
}

Future<void> _waitForIframe(JSAny? d, JSAny? iframe, String? initialHtml) {
  return (newPromise(((JSAny? resolve, JSAny? reject) {
        var isComplete = false;
        void succeed() {
          if (isComplete) return;
          isComplete = true;
          jsCall(resolve, jsUndefined);
        }

        void fail(JSAny? error) {
          if (isComplete) return;
          isComplete = true;
          jsCall(reject, jsUndefined, [error]);
        }

        setp(iframe, 'onload', (() {
          succeed();
        }).toJS);
        setp(iframe, 'onerror', ((JSAny? error) {
          fail(error);
        }).toJS);

        final style = getp(iframe, 'style');
        method(style, 'setProperty', ['display'.toJS, 'block'.toJS, 'important'.toJS]);
        setp(style, 'position', 'absolute'.toJS);
        setp(style, 'top', '0'.toJS);
        setp(style, 'left', '0'.toJS);
        setp(style, 'visibility', 'hidden'.toJS);

        if (initialHtml != null && hasIn(iframe, 'srcdoc')) {
          setp(iframe, 'srcdoc', initialHtml.toJS);
        } else {
          setp(iframe, 'src', 'about:blank'.toJS);
        }
        method(getp(d, 'body'), 'appendChild', [iframe]);

        void checkReadyState() {
          if (isComplete) return;
          final ready = getp(
            getp(getp(iframe, 'contentWindow'), 'document'),
            'readyState',
          );
          if (jsString(ready) == 'complete') {
            succeed();
          } else {
            jsSetTimeout((() {
              checkReadyState();
            }).toJS, 10);
          }
        }

        checkReadyState();
      }).toJS) as JSPromise<JSAny?>)
      .toDart;
}

JSAny? selectorToElement(String selector) {
  final (tag, attributes) = parseSimpleCssSelector(selector);
  final element = method(jsDocument, 'createElement', [(tag ?? 'div').toJS]);
  for (final name in attributes.keys) {
    final value = attributes[name]!.join(' ');
    if (name == 'style') {
      addStyleString(getp(element, 'style'), value);
    } else {
      method(element, 'setAttribute', [name.toJS, value.toJS]);
    }
  }
  return element;
}

void addStyleString(JSAny? style, String source) {
  for (final property in source.split(';')) {
    final match = RegExp(
      r'^\s*([\w-]+)\s*:\s*(.+?)(\s*!([\w-]+))?\s*$',
    ).firstMatch(property);
    if (match != null) {
      method(style, 'setProperty', [
        match.group(1)!.toJS,
        match.group(2)!.toJS,
        (match.group(4) ?? '').toJS,
      ]);
    }
  }
}

bool isAnyParentCrossOrigin() {
  var currentWindow = jsWindow;
  for (;;) {
    final parentWindow = getp(currentWindow, 'parent');
    if (parentWindow.isUndefinedOrNull || parentWindow == currentWindow) {
      return false;
    }
    final (ok, value) = tryHost(() {
      final parentOrigin = getp(getp(parentWindow, 'location'), 'origin');
      final currentOrigin = getp(getp(currentWindow, 'location'), 'origin');
      return fromDart(jsString(parentOrigin) != jsString(currentOrigin));
    });
    if (!ok) {
      if (isError(value) && jsString(getp(value, 'name')) == 'SecurityError') {
        return true;
      }
      throw value ?? makeError('Error', 'origin check failed')!;
    }
    if (jsTruthy(value)) return true;
    currentWindow = parentWindow;
  }
}

Future<JSAny?> _awaitJs(JSAny? promise) {
  if (!isPromise(promise)) return Future.value(promise);
  return (promise as JSPromise<JSAny?>).toDart;
}
