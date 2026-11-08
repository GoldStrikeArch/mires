library;

import 'dart:js_interop';

import 'browser.dart';
import 'data.dart';
import 'host.dart';

const commentTemplate =
    r'$ if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score';

JSAny? getConfidence(JSAny? components) {
  final open = getOpenConfidenceScore(components);
  final pro = roundTo(0.99 + 0.01 * open, 0.0001);
  final result = jsObject();
  setp(result, 'score', open.toJS);
  setp(
    result,
    'comment',
    commentTemplate.replaceAll(r'$', _trimNumber(pro)).toJS,
  );
  return result;
}

double getOpenConfidenceScore(JSAny? components) {
  if (isAndroid()) return 0.4;
  if (isWebKit()) {
    return isDesktopWebKit() && !(isWebKit616OrNewer() && isSafariWebKit())
        ? 0.5
        : 0.3;
  }
  final platformComponent = getp(components, 'platform');
  final platform = hasOwn(platformComponent, 'value')
      ? jsString(getp(platformComponent, 'value'))
      : '';
  if (RegExp(r'^Win').hasMatch(platform)) return 0.6;
  if (RegExp(r'^Mac').hasMatch(platform)) return 0.5;
  return 0.7;
}

String _trimNumber(double value) {
  final text = value.toString();
  if (text.contains('e') || text.contains('E')) return text;
  return text;
}
