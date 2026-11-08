import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:fingerprintjs_dart/agent.dart';
import 'package:fingerprintjs_dart/browser.dart';
import 'package:fingerprintjs_dart/dom.dart';
import 'package:fingerprintjs_dart/entropy.dart';
import 'package:fingerprintjs_dart/hashing.dart';
import 'package:fingerprintjs_dart/host.dart';
import 'package:fingerprintjs_dart/sources.dart';
import 'package:fingerprintjs_dart/sources_audio.dart';
import 'package:fingerprintjs_dart/sources_canvas.dart';
import 'package:fingerprintjs_dart/sources_frame.dart';
import 'package:fingerprintjs_dart/sources_simple.dart';
import 'package:fingerprintjs_dart/sources_webgl.dart';

void main() {
  retainWebImport();
  final sources = makeSources();
  final loadFn = dartFun1('load', load, arity: 0);
  final hashFn = dartFun1('hashComponents', hashComponents);
  final debugFn = dartFun1('componentsToDebugString', componentsToDebugString);
  final exports = jsObject();
  setp(exports, 'componentsToDebugString', debugFn);
  setp(exports, 'getFullscreenElement', dartFun0('getFullscreenElement', getFullscreenElement));
  setp(
    exports,
    'getUnstableAudioFingerprint',
    dartFun0('getUnstableAudioFingerprint', getUnstableAudioFingerprint),
  );
  setp(
    exports,
    'getUnstableCanvasFingerprint',
    dartFun1('getUnstableCanvasFingerprint', getUnstableCanvasFingerprint, arity: 0),
  );
  setp(
    exports,
    'getUnstableHardwareConcurrency',
    dartFun0('getUnstableHardwareConcurrency', getUnstableHardwareConcurrency),
  );
  setp(exports, 'getUnstableScreenFrame', dartFun0('getUnstableScreenFrame', getUnstableScreenFrame));
  setp(
    exports,
    'getUnstableScreenResolution',
    dartFun0('getUnstableScreenResolution', getUnstableScreenResolution),
  );
  setp(exports, 'getWebGLContext', dartFun1('getWebGLContext', getWebGLContextExport));
  setp(exports, 'hashComponents', hashFn);
  setp(exports, 'isAndroid', exportBool('isAndroid', isAndroid));
  setp(exports, 'isChromium', exportBool('isChromium', isChromium));
  setp(exports, 'isDesktopWebKit', exportBool('isDesktopWebKit', isDesktopWebKit));
  setp(exports, 'isEdgeHTML', exportBool('isEdgeHTML', isEdgeHTML));
  setp(exports, 'isGecko', exportBool('isGecko', isGecko));
  setp(exports, 'isSamsungInternet', exportBool('isSamsungInternet', isSamsungInternet));
  setp(exports, 'isTrident', exportBool('isTrident', isTrident));
  setp(exports, 'isWebKit', exportBool('isWebKit', isWebKit));
  setp(exports, 'load', loadFn);
  setp(exports, 'loadSources', dartFun4('loadSources', loadSourcesExport, arity: 3));
  setp(
    exports,
    'murmurX64Hash128',
    namedFun(
      'x64hash128',
      1,
      ((JSAny? input, JSAny? seed, JSAny? c, JSAny? d) {
        final text = input.isUndefinedOrNull ? '' : jsString(input);
        // JavaScript bitwise operations treat non-finite seeds as zero.
        final seedNumber = seed.isUndefinedOrNull ? 0.0 : jsNumber(seed);
        final actual = seedNumber.isFinite ? seedNumber.toInt() : 0;
        return x64hash128(text, actual).toJS;
      }).toJS,
    ),
  );
  setp(exports, 'prepareForSources', dartFun1('prepareForSources', prepareForSources, arity: 0));
  setp(exports, 'sources', sources);
  setp(exports, 'transformSource', dartFun2('transformSource', transformSource));
  setp(
    exports,
    'withIframe',
    namedFun(
      'withIframe',
      1,
      ((JSAny? action, JSAny? html, JSAny? poll, JSAny? d) {
        return withIframeExport(action, html, poll);
      }).toJS,
    ),
  );

  globalContext.setProperty('__fp_dart_exports'.toJS, exports);
}
