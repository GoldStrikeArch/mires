library;

import 'dart:js_interop';

import 'browser.dart';
import 'host.dart';

const statusNoGlContext = -1;
const statusGetParameterNotAFunction = -2;

const rendererInfoExtensionName = 'WEBGL_debug_renderer_info';
const polygonModeExtensionName = 'WEBGL_polygon_mode';

const shaderTypes = ['FRAGMENT_SHADER', 'VERTEX_SHADER'];
const precisionTypes = [
  'LOW_FLOAT',
  'MEDIUM_FLOAT',
  'HIGH_FLOAT',
  'LOW_INT',
  'MEDIUM_INT',
  'HIGH_INT',
];

final validContextParameters = <int>{
  10752, 2849, 2884, 2885, 2886, 2928, 2929, 2930, 2931, 2932, 2960, 2961, 2962,
  2963, 2964, 2965, 2966, 2967, 2968, 2978, 3024, 3042, 3088, 3089, 3106, 3107,
  32773, 32777, 32823, 32824, 32936, 32937, 32938, 32939, 32968, 32969, 32970,
  32971, 3317, 33170, 3333, 3379, 3386, 33901, 33902, 34016, 34024, 34076, 3408,
  3410, 3411, 3412, 3413, 3414, 3415, 34467, 34816, 34817, 34818, 34819, 34877,
  34921, 34930, 35660, 35661, 35724, 35738, 35739, 36003, 36004, 36005, 36347,
  36348, 36349, 37440, 37441, 37443, 7936, 7937, 7938,
};

final validExtensionParams = <int>{
  34047,
  35723,
  36063,
  34852,
  34853,
  34854,
  34229,
  36392,
  36795,
  38449,
};

JSAny? getWebGlBasics(JSAny? options) {
  final cache = getp(options, 'cache');
  final gl = getWebGLContext(cache);
  if (!jsTruthy(gl)) return statusNoGlContext.toJS;
  if (!_isValidParameterGetter(gl)) return statusGetParameterNotAFunction.toJS;
  final debugExtension = shouldAvoidDebugRendererInfo()
      ? jsNull
      : method(gl, 'getExtension', [rendererInfoExtensionName.toJS]);
  final result = jsObject();
  setp(result, 'version', _paramString(gl, getp(gl, 'VERSION')));
  setp(result, 'vendor', _paramString(gl, getp(gl, 'VENDOR')));
  setp(
    result,
    'vendorUnmasked',
    jsTruthy(debugExtension)
        ? _paramString(gl, getp(debugExtension, 'UNMASKED_VENDOR_WEBGL'))
        : ''.toJS,
  );
  setp(result, 'renderer', _paramString(gl, getp(gl, 'RENDERER')));
  setp(
    result,
    'rendererUnmasked',
    jsTruthy(debugExtension)
        ? _paramString(gl, getp(debugExtension, 'UNMASKED_RENDERER_WEBGL'))
        : ''.toJS,
  );
  setp(
    result,
    'shadingLanguageVersion',
    _paramString(gl, getp(gl, 'SHADING_LANGUAGE_VERSION')),
  );
  return result;
}

JSAny? getWebGlExtensions(JSAny? options) {
  final cache = getp(options, 'cache');
  final gl = getWebGLContext(cache);
  if (!jsTruthy(gl)) return statusNoGlContext.toJS;
  if (!_isValidParameterGetter(gl)) return statusGetParameterNotAFunction.toJS;

  final extensions = method(gl, 'getSupportedExtensions');
  final contextAttributes = method(gl, 'getContextAttributes');
  final unsupportedExtensions = jsArray();
  final attributes = jsArray();
  final parameters = jsArray();
  final extensionParameters = jsArray();
  final shaderPrecisions = jsArray();

  if (jsTruthy(contextAttributes)) {
    final keys = objectKeys(contextAttributes);
    for (var i = 0; i < keys.length; i++) {
      final name = jsString(jsIndex(keys, i));
      jsPush(
        attributes,
        '${name}=${jsString(getp(contextAttributes, name))}'.toJS,
      );
    }
  }

  final constants = _getConstantsFromPrototype(gl);
  for (var i = 0; i < constants.length; i++) {
    final constant = jsString(jsIndex(constants, i));
    final code = getp(gl, constant);
    final codeNum = jsNumber(code);
    final extra = typof(code) == 'number' &&
            validContextParameters.contains(codeNum.toInt())
        ? '=${jsString(method(gl, 'getParameter', [code]))}'
        : '';
    jsPush(parameters, '$constant=${jsString(code)}$extra'.toJS);
  }

  if (jsTruthy(extensions)) {
    for (var i = 0; i < jsLength(extensions); i++) {
      final name = jsString(jsIndex(extensions, i));
      if ((name == rendererInfoExtensionName && shouldAvoidDebugRendererInfo()) ||
          (name == polygonModeExtensionName && shouldAvoidPolygonModeExtensions())) {
        continue;
      }
      final extension = method(gl, 'getExtension', [name.toJS]);
      if (!jsTruthy(extension)) {
        jsPush(unsupportedExtensions, name.toJS);
        continue;
      }
      final extConstants = _getConstantsFromPrototype(extension);
      for (var j = 0; j < extConstants.length; j++) {
        final constant = jsString(jsIndex(extConstants, j));
        final code = getp(extension, constant);
        final extra = typof(code) == 'number' &&
                validExtensionParams.contains(jsNumber(code).toInt())
            ? '=${jsString(method(gl, 'getParameter', [code]))}'
            : '';
        jsPush(extensionParameters, '$constant=${jsString(code)}$extra'.toJS);
      }
    }
  }

  for (final shaderType in shaderTypes) {
    for (final precisionType in precisionTypes) {
      final precision = _getShaderPrecision(gl, shaderType, precisionType);
      jsPush(shaderPrecisions, '$shaderType.$precisionType=$precision'.toJS);
    }
  }

  method(extensionParameters, 'sort');
  method(parameters, 'sort');

  final result = jsObject();
  setp(result, 'contextAttributes', attributes);
  setp(result, 'parameters', parameters);
  setp(result, 'shaderPrecisions', shaderPrecisions);
  setp(result, 'extensions', extensions);
  setp(result, 'extensionParameters', extensionParameters);
  setp(result, 'unsupportedExtensions', unsupportedExtensions);
  return result;
}

JSAny? getWebGLContext(JSAny? cache) {
  final existing = getp(cache, 'webgl');
  if (jsTruthy(existing) || hasOwn(cache, 'webgl')) {
    if (existing.isDefinedAndNotNull) return getp(existing, 'context');
  }
  final canvas = method(jsDocument, 'createElement', ['canvas'.toJS]);
  JSAny? context = jsUndefined;
  method(canvas, 'addEventListener', [
    'webglCreateContextError'.toJS,
    (() {
      context = jsUndefined;
    }).toJS,
  ]);
  for (final type in ['webgl', 'experimental-webgl']) {
    final (ok, value) = tryHost(() => method(canvas, 'getContext', [type.toJS]));
    if (ok && jsTruthy(value)) {
      context = value;
      break;
    }
  }
  final entry = jsObject();
  setp(entry, 'context', context.isUndefined ? jsUndefined : context);
  setp(cache, 'webgl', entry);
  return context.isUndefined ? jsUndefined : context;
}

JSAny? getWebGLContextExport(JSAny? cache) {
  final target = cache.isUndefinedOrNull ? jsObject() : cache;
  return getWebGLContext(target);
}

JSAny? _paramString(JSAny? gl, JSAny? code) {
  final value = method(gl, 'getParameter', [code]);
  if (value.isUndefinedOrNull) return ''.toJS;
  return fromDart(jsString(value));
}

JSArray<JSAny?> _getConstantsFromPrototype(JSAny? obj) {
  final proto = getProto(obj);
  final keys = objectKeys(proto);
  final result = jsArray();
  for (var i = 0; i < keys.length; i++) {
    final key = jsIndex(keys, i);
    if (typof(key) == 'string' &&
        !jsTest(jsRegex(r'[^A-Z0-9_x]'), jsString(key))) {
      jsPush(result, key);
    }
  }
  return result;
}

String _getShaderPrecision(JSAny? gl, String shaderType, String precisionType) {
  final format = method(gl, 'getShaderPrecisionFormat', [
    getp(gl, shaderType),
    getp(gl, precisionType),
  ]);
  if (!jsTruthy(format)) return '';
  return '${jsString(getp(format, 'rangeMin'))},${jsString(getp(format, 'rangeMax'))},${jsString(getp(format, 'precision'))}';
}

bool shouldAvoidDebugRendererInfo() => isGecko();

bool shouldAvoidPolygonModeExtensions() => isChromium() || isWebKit();

bool _isValidParameterGetter(JSAny? gl) =>
    typof(getp(gl, 'getParameter')) == 'function';
