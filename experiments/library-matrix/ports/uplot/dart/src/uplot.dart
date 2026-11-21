// The slice of uPlot 1.6.32 (node_modules/uplot/dist/uPlot.esm.js) that the reference chart runs:
// the rounding helpers, calcSize's plot box, the value/pixel conversions, setScales' index window,
// paths.linear with findGaps and clipGaps, and drawPath. As in the reference, the x scale has
// ori 0 and the y scale ori 1, both dir 1, and the series has pxAlign 1, alignGaps 0, width 1,
// no fill, no bands and no points.
import 'dart:math' as math;

import 'dom.dart';

/// Number.EPSILON, 2^-52.
const _epsilon = 2.220446049250313e-16;

/// Number.isInteger.
bool _isInt(double value) => value.isFinite && value.floorToDouble() == value;

/// roundDec: rounds half away from zero at [dec] decimals.
double roundDec(double value, [int dec = 0]) {
  if (_isInt(value)) return value;
  final p = math.pow(10, dec).toDouble();
  final n = value * p * (1 + _epsilon);
  return jsRound(n) / p;
}

/// uPlot's regex6: the decimals before a run of six or more 9s or 0s.
final _regex6 = RegExp(r'\.\d*?(?=9{6,}|0{6,})');

/// fixFloat: snaps a value whose JS string has a run of six or more 9s or 0s after the point to the
/// decimals before the run (17999.204999999998 becomes 17999.205). uPlot first returns members of
/// its fixedDec map unchanged; the string path returns every member unchanged too. Its branch for
/// strings with `e-` (values below 1e-6) is not reached by plot sizes of 1 px or more.
double fixFloat(double value) {
  if (_isInt(value)) return value;
  // On the web a non-integral double's toString() is the JS string.
  final match = _regex6.firstMatch(value.toString());
  if (match == null) return value;
  return roundDec(value, match[0]!.length - 1);
}

double incrRound(double num, double incr) => fixFloat(roundDec(fixFloat(num / incr)) * incr);

/// closestIdx: the index of the value in [arr] closest to [num]. JS `lo || 0` and
/// `hi || arr.length - 1` make a zero [hi] the last index.
int closestIdx(double num, List<double> arr, [int lo = 0, int hi = 0]) {
  var low = lo;
  var high = hi != 0 ? hi : arr.length - 1;
  while (high - low > 1) {
    final mid = (low + high) >> 1;
    if (arr[mid] < num) {
      low = mid;
    } else {
      high = mid;
    }
  }
  // An empty array reads undefined on both sides, so the comparison fails and `hi` (-1) wins.
  if (arr.isNotEmpty && num - arr[low] <= arr[high] - num) return low;
  return high;
}

/// A linear scale after setScale: uPlot's `_min` and `_max`.
class Scale {
  final double min, max;

  Scale(this.min, this.max);

  double valToPct(double value) => (value - min) / (max - min);
}

/// The plot box. uPlot gets CSS dimensions (the adapter divides the physical size by pxRatio) and
/// calcSize multiplies them back, rounding to half pixels with incrRound. Without padding or axes
/// the box starts at the canvas origin.
class Plot {
  static const leftCss = 0.0, topCss = 0.0;
  final double pxRatio, widthCss, heightCss;
  final double left, top, width, height;

  Plot(this.pxRatio, this.widthCss, this.heightCss)
      : left = incrRound(leftCss * pxRatio, 0.5),
        top = incrRound(topCss * pxRatio, 0.5),
        width = incrRound(widthCss * pxRatio, 0.5),
        height = incrRound(heightCss * pxRatio, 0.5);
}

/// One frame's chart: what uPlot's valToPos, posToVal, valToIdx and posToIdx read.
class Chart {
  final Plot plot;
  final Scale x, y;
  final List<double> xs;

  // setScales' visible window. With no data uPlot leaves it null, which closestIdx reads as 0.
  int _i0 = 0, _i1 = 0;

  Chart(this.plot, this.x, this.y, this.xs) {
    if (xs.isNotEmpty) {
      _i0 = closestIdx(x.min, xs);
      _i1 = closestIdx(x.max, xs);
      // Don't contract same or adjacent indices; the closest ones can be outside the view.
      if (_i1 - _i0 > 1) {
        if (xs[_i0] < x.min) _i0++;
        if (xs[_i1] > x.max) _i1--;
      }
    }
  }

  /// getHPos (ori 0, dir 1) in canvas pixels: valToPos(value, 'x', true).
  double valToPosX(double value) => plot.left + plot.width * x.valToPct(value);

  /// getVPos (ori 1, dir 1) in canvas pixels: valToPos(value, 'y', true).
  double valToPosY(double value) => plot.top + plot.height * (1 - y.valToPct(value));

  /// posToVal(pos, 'x', true).
  double posToValX(double pos) {
    final css = pos / plot.pxRatio - Plot.leftCss;
    return x.min + (x.max - x.min) * (css / plot.widthCss);
  }

  /// posToVal(pos, 'y', true): ori 1 measures from the bottom.
  double posToValY(double pos) {
    final css = pos / plot.pxRatio - Plot.topCss;
    final dim = plot.heightCss;
    return y.min + (y.max - y.min) * ((dim - css) / dim);
  }

  int valToIdx(double value) => closestIdx(value, xs);

  int posToIdx(double pos) => closestIdx(posToValX(pos), xs, _i0, _i1);
}

/// The stroke path, recording each lineTo as the adapter's RecordedPath does.
class RecordedPath {
  final path = Path2D();
  final vertices = <(double, double)>[];

  void lineTo(double x, double y) {
    vertices.add((x, y));
    path.lineTo(x, y);
  }
}

/// What paths.linear returns, less the parts this chart never sets (fill, band). [gaps] and [clip]
/// are null with spanGaps; [clip] is also null without gaps.
class SeriesPaths {
  final RecordedPath stroke;
  final List<(double, double)>? gaps;
  final Path2D? clip;

  SeriesPaths(this.stroke, this.gaps, this.clip);
}

/// One pixel column while decimating: uPlot's inY, outY, minY and maxY, which exist while
/// minY != null.
class _Accumulator {
  final double inY;
  double outY, minY, maxY;

  _Accumulator(this.inY)
      : outY = inY,
        minY = inY,
        maxY = inY;

  void add(double y) {
    outY = y;
    if (y < minY) {
      minY = y;
    } else if (y > maxY) {
      maxY = y;
    }
  }
}

/// _drawAcc with lineToH: a column's extremes, then its last value, all in pixels.
void _drawAcc(RecordedPath stroke, double accX, double minY, double maxY, double inY, double outY) {
  if (minY != maxY) {
    if (inY != minY && outY != minY) stroke.lineTo(accX, minY);
    if (inY != maxY && outY != maxY) stroke.lineTo(accX, maxY);
    stroke.lineTo(accX, outY);
  }
}

/// findGaps for dir 1 and alignGaps 0: a run of nulls spans from the pixel of the value before it
/// to the pixel of the value after it. Both neighbours exist, since idx0 and idx1 hold values, so
/// uPlot's fallbacks to the run's own ends never apply.
List<(double, double)> _findGaps(
    List<double> xs, List<double?> ys, int idx0, int idx1, double Function(double) pixelForX) {
  final gaps = <(double, double)>[];
  for (var i = idx0; i <= idx1; i++) {
    if (ys[i] == null) {
      final fr = i;
      var to = i;
      while (++i <= idx1 && ys[i] == null) {
        to = i;
      }
      final frPx = pixelForX(xs[fr - 1]);
      final toPx = pixelForX(xs[to + 1]);
      if (toPx >= frPx) gaps.add((frPx, toPx));
    }
  }
  return gaps;
}

/// clipGaps for ori 0: rectangles over everything but the gaps.
Path2D? _clipGaps(List<(double, double)> gaps, Plot plot) {
  if (gaps.isEmpty) return null;
  final clip = Path2D();
  var prevGapEnd = plot.left;
  for (final (from, to) in gaps) {
    if (to > from) {
      final w = from - prevGapEnd;
      if (w > 0) clip.rect(prevGapEnd, plot.top, w, plot.top + plot.height);
      prevGapEnd = to;
    }
  }
  final w = plot.left + plot.width - prevGapEnd;
  // uPlot's hack to keep strokes at the edges from being cut off.
  const maxStrokeWidth = 10.0;
  if (w > 0) clip.rect(prevGapEnd, plot.top - maxStrokeWidth / 2, w, plot.top + plot.height + maxStrokeWidth);
  return clip;
}

/// paths.linear's decimating loop: one vertex per value until a pixel column holds two or more.
/// Returns hasGap.
bool _strokeDecimated(Chart u, RecordedPath stroke, List<double?> dataY, int idx0, int idx1,
    double Function(double) pixelForX, double Function(double) pixelForY) {
  final dataX = u.xs;
  double xForPixel(double pos) => u.posToValX(pos);
  var hasGap = false;
  _Accumulator? acc;
  var accX = pixelForX(dataX[idx0]);
  final idx0px = pixelForX(dataX[idx0]);
  // The limit of the current x bucket, to avoid computing the pixel of every x value.
  var nextAccXVal = xForPixel(idx0px + 1);

  void flush(_Accumulator a) =>
      _drawAcc(stroke, accX, pixelForY(a.minY), pixelForY(a.maxY), pixelForY(a.inY), pixelForY(a.outY));

  for (var i = idx0; i <= idx1; i++) {
    final xVal = dataX[i];
    final reuseAccX = xVal < nextAccXVal;
    final x = reuseAccX ? accX : pixelForX(xVal);
    final yVal = dataY[i];

    if (x == accX) {
      if (yVal != null) {
        if (acc == null) {
          stroke.lineTo(x, pixelForY(yVal));
          acc = _Accumulator(yVal);
        } else {
          acc.add(yVal);
        }
      } else {
        hasGap = true;
      }
    } else {
      if (acc != null) flush(acc);
      if (yVal != null) {
        stroke.lineTo(x, pixelForY(yVal));
        acc = _Accumulator(yVal);
      } else {
        acc = null;
        hasGap = true;
      }
      accX = x;
      nextAccXVal = xForPixel(accX + 1);
    }
  }

  if (acc != null && acc.minY != acc.maxY) flush(acc);
  return hasGap;
}

/// paths.linear for one series over the whole data range, as the adapter calls it.
SeriesPaths linear(Chart u, List<double?> dataY, bool spanGaps) {
  final dataX = u.xs;
  // nonNullIdxs over the whole range: the first and last indices holding a value, or -1.
  final idx0 = dataY.indexWhere((v) => v != null);
  final idx1 = dataY.lastIndexWhere((v) => v != null);
  double pixelForX(double value) => jsRound(u.valToPosX(value));
  double pixelForY(double value) => jsRound(u.valToPosY(value));
  final stroke = RecordedPath();
  // Decimate when the number of points is at least 4x the available pixels.
  final decimate = idx1 - idx0 >= u.plot.width * 4;

  final bool hasGap;
  if (idx0 < 0) {
    // An all-null series has idx0 = idx1 = -1, where uPlot's loop reads undefined and draws nothing.
    hasGap = false;
  } else if (decimate) {
    hasGap = _strokeDecimated(u, stroke, dataY, idx0, idx1, pixelForX, pixelForY);
  } else {
    var nulls = false;
    for (var i = idx0; i <= idx1; i++) {
      final yVal = dataY[i];
      if (yVal == null) {
        nulls = true;
      } else {
        stroke.lineTo(pixelForX(dataX[i]), pixelForY(yVal));
      }
    }
    hasGap = nulls;
  }

  if (spanGaps) return SeriesPaths(stroke, null, null);
  // series.gaps is the identity.
  final gaps = hasGap ? _findGaps(dataX, dataY, idx0, idx1, pixelForX) : <(double, double)>[];
  return SeriesPaths(stroke, gaps, _clipGaps(gaps, u.plot));
}

/// The adapter's series options: stroke colour and width.
const _seriesStroke = '#172839';
const _seriesWidth = 1.0;

/// drawPath for a line series, with strokeFill's clips and setCtxStyle's defaults.
void drawPath(Context2D ctx, Plot plot, SeriesPaths paths) {
  final width = roundDec(_seriesWidth * plot.pxRatio, 3);
  // JS %: remainder, not Dart's Euclidean %.
  final offset = width.remainder(2) / 2;
  final pxAlign = offset > 0;

  if (pxAlign) ctx.translate(offset, offset);

  final boundsClip = Path2D()
    ..rect(plot.left - width / 2, plot.top - width / 2, plot.width + width, plot.height + width);

  ctx
    ..strokeStyle = _seriesStroke
    ..lineWidth = width
    ..lineJoin = 'round'
    ..lineCap = 'butt'
    ..save()
    ..clip(boundsClip);
  final clip = paths.clip;
  if (clip != null) ctx.clip(clip);
  if (width > 0) ctx.stroke(paths.stroke.path);
  ctx.restore();

  if (pxAlign) ctx.translate(-offset, -offset);
}
