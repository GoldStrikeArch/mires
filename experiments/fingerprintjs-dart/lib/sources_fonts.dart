library;

import 'dart:js_interop';

import 'dom.dart';
import 'fp_async.dart';
import 'host.dart';

const testString = 'mmMwWLliI0O&1';
const textSize = '48px';
const baseFonts = ['monospace', 'sans-serif', 'serif'];
const fontList = [
  'sans-serif-thin',
  'ARNO PRO',
  'Agency FB',
  'Arabic Typesetting',
  'Arial Unicode MS',
  'AvantGarde Bk BT',
  'BankGothic Md BT',
  'Batang',
  'Bitstream Vera Sans Mono',
  'Calibri',
  'Century',
  'Century Gothic',
  'Clarendon',
  'EUROSTILE',
  'Franklin Gothic',
  'Futura Bk BT',
  'Futura Md BT',
  'GOTHAM',
  'Gill Sans',
  'HELV',
  'Haettenschweiler',
  'Helvetica Neue',
  'Humanst521 BT',
  'Leelawadee',
  'Letter Gothic',
  'Levenim MT',
  'Lucida Bright',
  'Lucida Sans',
  'Menlo',
  'MS Mincho',
  'MS Outlook',
  'MS Reference Specialty',
  'MS UI Gothic',
  'MT Extra',
  'MYRIAD PRO',
  'Marlett',
  'Meiryo UI',
  'Microsoft Uighur',
  'Minion Pro',
  'Monotype Corsiva',
  'PMingLiU',
  'Pristina',
  'SCRIPTINA',
  'Segoe UI Light',
  'Serifa',
  'SimHei',
  'Small Fonts',
  'Staccato222 BT',
  'TRAJAN PRO',
  'Univers CE 55 Medium',
  'Vrinda',
  'ZWAdobeF',
];

JSAny? getFonts([JSAny? _]) {
  return futureToPromise(withIframe(((JSAny? iframe, JSAny? iWindow) {
    final document = getp(iWindow, 'document');
    final holder = getp(document, 'body');
    setp(getp(holder, 'style'), 'fontSize', textSize.toJS);

    final spansContainer = method(document, 'createElement', ['div'.toJS]);
    method(getp(spansContainer, 'style'), 'setProperty', [
      'visibility'.toJS,
      'hidden'.toJS,
      'important'.toJS,
    ]);

    final defaultWidth = jsObject();
    final defaultHeight = jsObject();

    JSAny? createSpan(String fontFamily) {
      final span = method(document, 'createElement', ['span'.toJS]);
      final style = getp(span, 'style');
      setp(style, 'position', 'absolute'.toJS);
      setp(style, 'top', '0'.toJS);
      setp(style, 'left', '0'.toJS);
      setp(style, 'fontFamily', fontFamily.toJS);
      setp(span, 'textContent', testString.toJS);
      method(spansContainer, 'appendChild', [span]);
      return span;
    }

    JSAny? createSpanWithFonts(String fontToDetect, String baseFont) {
      return createSpan("'$fontToDetect',$baseFont");
    }

    final baseFontsSpans = jsArray();
    for (final font in baseFonts) {
      jsPush(baseFontsSpans, createSpan(font));
    }

    final fontsSpans = jsObject();
    for (final font in fontList) {
      final spans = jsArray();
      for (final baseFont in baseFonts) {
        jsPush(spans, createSpanWithFonts(font, baseFont));
      }
      setp(fontsSpans, font, spans);
    }

    method(holder, 'appendChild', [spansContainer]);

    for (var index = 0; index < baseFonts.length; index++) {
      setp(defaultWidth, baseFonts[index], getp(jsIndex(baseFontsSpans, index), 'offsetWidth'));
      setp(defaultHeight, baseFonts[index], getp(jsIndex(baseFontsSpans, index), 'offsetHeight'));
    }

    final available = jsArray();
    for (final font in fontList) {
      final fontSpans = getp(fontsSpans, font);
      var found = false;
      for (var i = 0; i < baseFonts.length; i++) {
        final span = jsIndex(fontSpans, i);
        if (jsNumber(getp(span, 'offsetWidth')) !=
                jsNumber(getp(defaultWidth, baseFonts[i])) ||
            jsNumber(getp(span, 'offsetHeight')) !=
                jsNumber(getp(defaultHeight, baseFonts[i]))) {
          found = true;
          break;
        }
      }
      if (found) jsPush(available, font.toJS);
    }
    return available;
  }).toJS));
}
