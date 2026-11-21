// The slice of uPlot 1.6.32 (node_modules/uplot/dist/uPlot.esm.js) that the reference chart runs:
// the rounding helpers, calcSize's plot box, the value/pixel conversions, setScales' index window,
// paths.linear with findGaps and clipGaps, and drawPath. As in the reference, the x scale has
// ori 0 and the y scale ori 1, both dir 1, and the series has pxAlign 1, alignGaps 0, width 1,
// no fill, no bands and no points.
package main

import (
	"math"

	"github.com/gopherjs/gopherjs/js"
)

// epsilon is Number.EPSILON.
const epsilon = 0x1p-52

// round is JS Math.round: the nearest integer, halves toward +Inf, and -0 for a negative value
// that rounds to zero. math.Round rounds halves away from zero, and floor(x+0.5) is wrong for
// 0.49999999999999994 and for odd integers past 2^52. By Sterbenz's lemma x-floor(x) is exact for
// x >= 0 and x <= -0.5; for x in (-0.5, 0) it can round up to 1 (e.g. -2^-60 + 1 == 1), which
// still increments r from -1 to 0 and returns -0 via the sign check, matching Math.round.
func round(x float64) float64 {
	r := math.Floor(x)
	if x-r >= 0.5 {
		r++
	}
	if r == 0 && x < 0 {
		return math.Copysign(0, -1)
	}
	return r
}

// isInt is Number.isInteger.
func isInt(v float64) bool { return !math.IsInf(v, 0) && math.Floor(v) == v }

// roundDec rounds half away from zero at dec decimals.
func roundDec(val float64, dec int) float64 {
	if isInt(val) {
		return val
	}
	p := math.Pow(10, float64(dec))
	n := val * p * (1 + epsilon)
	return round(n) / p
}

// decimalsBeforeRun is uPlot's regex6, /\.\d*?(?=9{6,}|0{6,})/: the number of digits between the
// point and the first run of six or more 9s or 0s, if there is one. Go's regexp has no lookahead,
// so this scans the digits after the point.
func decimalsBeforeRun(s string) (int, bool) {
	dot := 0
	for dot < len(s) && s[dot] != '.' {
		dot++
	}
	if dot == len(s) {
		return 0, false
	}
	end := dot + 1
	for end < len(s) && '0' <= s[end] && s[end] <= '9' {
		end++
	}
	for k := dot + 1; k+6 <= end; k++ {
		if run := s[k : k+6]; run == "999999" || run == "000000" {
			return k - dot - 1, true
		}
	}
	return 0, false
}

func findNegExp(s string) int {
	for i := 0; i+1 < len(s); i++ {
		if s[i] == 'e' && s[i+1] == '-' {
			return i
		}
	}
	return -1
}

// fixFloat snaps a value whose JS string has a run of six or more 9s or 0s after the point to the
// decimals before the run (17999.204999999998 becomes 17999.205), handling "e-" exponent strings
// by rounding the mantissa. uPlot first returns members of its fixedDec map unchanged; the string
// path returns every member unchanged too.
func fixFloat(val float64) float64 {
	if isInt(val) {
		return val
	}
	str := jsString(val)
	dec, ok := decimalsBeforeRun(str)
	if !ok {
		return val
	}
	if ePos := findNegExp(str); ePos != -1 {
		mant := jsNumber(str[:ePos])
		return jsNumber(jsString(roundDec(mant, dec)) + str[ePos:])
	}
	return roundDec(val, dec)
}

func incrRound(num, incr float64) float64   { return fixFloat(roundDec(fixFloat(num/incr), 0) * incr) }
func incrRoundUp(num, incr float64) float64 { return fixFloat(math.Ceil(fixFloat(num/incr)) * incr) }
func incrRoundDn(num, incr float64) float64 { return fixFloat(math.Floor(fixFloat(num/incr)) * incr) }

// rangeNum is uPlot's rangeNum(min, max, 0.1, true): used by autoScaleX when all x values are equal.
func rangeNum(minV, maxV float64) (float64, float64) {
	delta := maxV - minV
	scalarMax := math.Max(math.Abs(minV), math.Abs(maxV))
	if delta < 1e-24 || math.Abs(jsLog10(scalarMax)-jsLog10(delta)) > 10 {
		delta = 0
		if minV == 0 || maxV == 0 {
			delta = 1e-24
		}
	}
	nonZeroDelta := delta
	if nonZeroDelta == 0 {
		if scalarMax != 0 {
			nonZeroDelta = scalarMax
		} else {
			nonZeroDelta = 1e3
		}
	}
	base := math.Pow(10, math.Floor(jsLog10(nonZeroDelta)))
	padMinFactor := 0.1
	if delta == 0 && minV != 0 {
		padMinFactor = 1
	}
	newMin := roundDec(incrRoundDn(minV-nonZeroDelta*padMinFactor, base/10), 24)
	softMin := math.Inf(1)
	if minV >= 0 && newMin <= 0 {
		softMin = 0
	}
	minLim := math.Min(softMin, newMin)
	if newMin < softMin && minV >= softMin {
		minLim = softMin
	}
	padMaxFactor := 0.1
	if delta == 0 && maxV != 0 {
		padMaxFactor = 1
	}
	newMax := roundDec(incrRoundUp(maxV+nonZeroDelta*padMaxFactor, base/10), 24)
	softMax := math.Inf(-1)
	if maxV <= 0 && newMax >= 0 {
		softMax = 0
	}
	maxLim := math.Max(softMax, newMax)
	if newMax > softMax && maxV <= softMax {
		maxLim = softMax
	}
	if minLim == maxLim && minLim == 0 {
		maxLim = 100
	}
	return minLim, maxLim
}

// closestIdx is the index of the value in arr closest to num. JS `lo || 0` and
// `hi || arr.length - 1` make a zero hi the last index.
func closestIdx(num float64, arr []float64, lo, hi int) int {
	if hi == 0 {
		hi = len(arr) - 1
	}
	for hi-lo > 1 {
		mid := (lo + hi) >> 1
		if arr[mid] < num {
			lo = mid
		} else {
			hi = mid
		}
	}
	// An empty array reads undefined on both sides, so the comparison fails and hi (-1) wins.
	if len(arr) > 0 && num-arr[lo] <= arr[hi]-num {
		return lo
	}
	return hi
}

// scale is a linear scale after setScale: uPlot's _min and _max.
type scale struct{ min, max float64 }

func (s scale) valToPct(v float64) float64 { return (v - s.min) / (s.max - s.min) }

// plot is the plot box. uPlot gets CSS dimensions (the adapter divides the physical size by
// pxRatio) and calcSize multiplies them back, rounding to half pixels with incrRound. Without
// padding or axes the box starts at the canvas origin.
type plot struct {
	pxRatio, leftCss, topCss, widthCss, heightCss float64
	left, top, width, height                      float64
}

func newPlot(pxRatio, widthCss, heightCss float64) plot {
	p := plot{pxRatio: pxRatio, widthCss: widthCss, heightCss: heightCss}
	p.left = incrRound(p.leftCss*pxRatio, 0.5)
	p.top = incrRound(p.topCss*pxRatio, 0.5)
	p.width = incrRound(widthCss*pxRatio, 0.5)
	p.height = incrRound(heightCss*pxRatio, 0.5)
	return p
}

// scaleState persists x and y scale bounds across frames for uPlot's setScale (< 1e-16) guard.
type scaleState struct {
	x, y *scale
}

// chart is one frame's chart: what uPlot's valToPos, posToVal, valToIdx and posToIdx read.
type chart struct {
	plot plot
	x, y scale
	xs   []float64
	// setScales' visible window. With no data uPlot leaves it null, which closestIdx reads as 0.
	i0, i1 int
}

// applyScales mirrors setData -> autoScaleX, explicit setScale('x'/'y'), and commit -> setScales.
// Unset scales default to scale{0, 0}, matching JS arithmetic on null scale bounds.
func (st *scaleState) applyScales(p plot, xReq, yReq scale, xs []float64) *chart {
	n := len(xs)
	lo, hi := 0, 0
	if n > 0 {
		hi = n - 1
	}
	pendX := true
	if n > 1 && xReq.max-xReq.min < 1e-16 {
		aMin, aMax := xs[0], xs[n-1]
		if aMin == aMax {
			aMin, aMax = rangeNum(aMin, aMax)
		}
		if aMin > aMax {
			aMin, aMax = aMax, aMin
		}
		pendX = !(aMax-aMin < 1e-16)
	}
	pendY := !(n > 1 && yReq.max-yReq.min < 1e-16)

	if pendX || pendY {
		if !pendX && st.x == nil {
			pendX = true
		}
		if !pendY && (st.y == nil || pendX) {
			pendY = true
		}
		if pendX {
			sx := xReq
			st.x = &sx
			if n > 0 {
				lo = closestIdx(xReq.min, xs, 0, 0)
				hi = closestIdx(xReq.max, xs, 0, 0)
				// Don't contract same or adjacent indices; the closest ones can be outside the view.
				if hi-lo > 1 {
					if xs[lo] < xReq.min {
						lo++
					}
					if xs[hi] > xReq.max {
						hi--
					}
				}
			}
		}
		if pendY {
			sy := yReq
			st.y = &sy
		}
	}

	var activeX, activeY scale
	if st.x != nil {
		activeX = *st.x
	}
	if st.y != nil {
		activeY = *st.y
	}
	return &chart{plot: p, x: activeX, y: activeY, xs: xs, i0: lo, i1: hi}
}

// valToPosX is getHPos (ori 0, dir 1) in canvas pixels: valToPos(v, "x", true).
func (u *chart) valToPosX(v float64) float64 { return u.plot.left + u.plot.width*u.x.valToPct(v) }

// valToPosY is getVPos (ori 1, dir 1) in canvas pixels: valToPos(v, "y", true).
func (u *chart) valToPosY(v float64) float64 {
	return u.plot.top + u.plot.height*(1-u.y.valToPct(v))
}

// posToValX is posToVal(pos, "x", true).
func (u *chart) posToValX(pos float64) float64 {
	css := pos/u.plot.pxRatio - u.plot.leftCss
	return u.x.min + (u.x.max-u.x.min)*(css/u.plot.widthCss)
}

// posToValY is posToVal(pos, "y", true): ori 1 measures from the bottom.
func (u *chart) posToValY(pos float64) float64 {
	css := pos/u.plot.pxRatio - u.plot.topCss
	dim := u.plot.heightCss
	return u.y.min + (u.y.max-u.y.min)*((dim-css)/dim)
}

func (u *chart) valToIdx(v float64) int { return closestIdx(v, u.xs, 0, 0) }

func (u *chart) posToIdx(pos float64) int { return closestIdx(u.posToValX(pos), u.xs, u.i0, u.i1) }

// pixelForX and pixelForY are paths.linear's pxRound(valToPos(...)) with pxAlign 1.
func (u *chart) pixelForX(v float64) float64 { return round(u.valToPosX(v)) }

func (u *chart) pixelForY(v float64) float64 { return round(u.valToPosY(v)) }

// recordedPath is the stroke path, recording each lineTo as the adapter's RecordedPath does.
type recordedPath struct {
	path     *js.Object
	vertices [][2]float64
}

func newPath2D() *js.Object { return js.Global.Get("Path2D").New() }

func (p *recordedPath) lineTo(x, y float64) {
	p.vertices = append(p.vertices, [2]float64{x, y})
	p.path.Call("lineTo", x, y)
}

// seriesPaths is what paths.linear returns, less the parts this chart never sets (fill, band).
// gaps and clip are nil with spanGaps; clip is also nil without gaps.
type seriesPaths struct {
	stroke *recordedPath
	gaps   [][2]float64
	clip   *js.Object
}

// accumulator is one pixel column while decimating: uPlot's inY, outY, minY and maxY, which exist
// while minY != null.
type accumulator struct{ inY, outY, minY, maxY float64 }

func (a *accumulator) add(y float64) {
	a.outY = y
	if y < a.minY {
		a.minY = y
	} else if y > a.maxY {
		a.maxY = y
	}
}

// drawAcc is _drawAcc with lineToH: a column's extremes, then its last value, all in pixels.
func (u *chart) drawAcc(stroke *recordedPath, accX float64, a *accumulator) {
	minY, maxY := u.pixelForY(a.minY), u.pixelForY(a.maxY)
	inY, outY := u.pixelForY(a.inY), u.pixelForY(a.outY)
	if minY != maxY {
		if inY != minY && outY != minY {
			stroke.lineTo(accX, minY)
		}
		if inY != maxY && outY != maxY {
			stroke.lineTo(accX, maxY)
		}
		stroke.lineTo(accX, outY)
	}
}

// findGaps for dir 1 and alignGaps 0: a run of nulls spans from the pixel of the value before it
// to the pixel of the value after it. Both neighbours exist, since idx0 and idx1 hold values, so
// uPlot's fallbacks to the run's own ends never apply.
func (u *chart) findGaps(ys []*float64, idx0, idx1 int) [][2]float64 {
	gaps := [][2]float64{}
	for i := idx0; i <= idx1; i++ {
		if ys[i] == nil {
			fr, to := i, i
			for i++; i <= idx1 && ys[i] == nil; i++ {
				to = i
			}
			frPx := u.pixelForX(u.xs[fr-1])
			toPx := u.pixelForX(u.xs[to+1])
			if toPx >= frPx {
				gaps = append(gaps, [2]float64{frPx, toPx})
			}
		}
	}
	return gaps
}

// clipGaps for ori 0: rectangles over everything but the gaps.
func clipGaps(gaps [][2]float64, p plot) *js.Object {
	if len(gaps) == 0 {
		return nil
	}
	clip := newPath2D()
	prevGapEnd := p.left
	for _, g := range gaps {
		if g[1] > g[0] {
			if w := g[0] - prevGapEnd; w > 0 {
				clip.Call("rect", prevGapEnd, p.top, w, p.top+p.height)
			}
			prevGapEnd = g[1]
		}
	}
	// uPlot's hack to keep strokes at the edges from being cut off.
	const maxStrokeWidth = 10.0
	if w := p.left + p.width - prevGapEnd; w > 0 {
		clip.Call("rect", prevGapEnd, p.top-maxStrokeWidth/2, w, p.top+p.height+maxStrokeWidth)
	}
	return clip
}

// strokeDecimated is paths.linear's decimating loop: one vertex per value until a pixel column
// holds two or more. It reports whether it met a null (hasGap).
func (u *chart) strokeDecimated(stroke *recordedPath, dataY []*float64, idx0, idx1 int) bool {
	dataX := u.xs
	hasGap := false
	var acc *accumulator
	accX := u.pixelForX(dataX[idx0])
	idx0px := u.pixelForX(dataX[idx0])
	// The limit of the current x bucket, to avoid computing the pixel of every x value.
	nextAccXVal := u.posToValX(idx0px + 1)

	for i := idx0; i <= idx1; i++ {
		xVal := dataX[i]
		reuseAccX := xVal < nextAccXVal
		x := accX
		if !reuseAccX {
			x = u.pixelForX(xVal)
		}
		yVal := dataY[i]

		if x == accX {
			switch {
			case yVal == nil:
				hasGap = true
			case acc == nil:
				stroke.lineTo(x, u.pixelForY(*yVal))
				acc = &accumulator{*yVal, *yVal, *yVal, *yVal}
			default:
				acc.add(*yVal)
			}
		} else {
			if acc != nil {
				u.drawAcc(stroke, accX, acc)
			}
			if yVal != nil {
				stroke.lineTo(x, u.pixelForY(*yVal))
				acc = &accumulator{*yVal, *yVal, *yVal, *yVal}
			} else {
				acc = nil
				hasGap = true
			}
			accX = x
			nextAccXVal = u.posToValX(accX + 1)
		}
	}

	if acc != nil && acc.minY != acc.maxY {
		u.drawAcc(stroke, accX, acc)
	}
	return hasGap
}

// nonNullIdxs over the whole range: the first and last indices holding a value, or -1.
func nonNullIdxs(data []*float64) (int, int) {
	first, last := -1, -1
	for i, v := range data {
		if v != nil {
			first = i
			break
		}
	}
	for i := len(data) - 1; i >= 0; i-- {
		if data[i] != nil {
			last = i
			break
		}
	}
	return first, last
}

// linear is paths.linear for one series over the whole data range, as the adapter calls it.
func (u *chart) linear(dataY []*float64, spanGaps bool) seriesPaths {
	idx0, idx1 := nonNullIdxs(dataY)
	stroke := &recordedPath{path: newPath2D(), vertices: [][2]float64{}}
	// Decimate when the number of points is at least 4x the available pixels.
	decimate := float64(idx1-idx0) >= u.plot.width*4

	hasGap := false
	switch {
	case idx0 < 0:
		// An all-null series has idx0 = idx1 = -1, where uPlot's loop reads undefined and draws
		// nothing.
	case decimate:
		hasGap = u.strokeDecimated(stroke, dataY, idx0, idx1)
	default:
		for i := idx0; i <= idx1; i++ {
			if yVal := dataY[i]; yVal == nil {
				hasGap = true
			} else {
				stroke.lineTo(u.pixelForX(u.xs[i]), u.pixelForY(*yVal))
			}
		}
	}

	if spanGaps {
		return seriesPaths{stroke: stroke}
	}
	// series.gaps is the identity.
	gaps := [][2]float64{}
	if hasGap {
		gaps = u.findGaps(dataY, idx0, idx1)
	}
	return seriesPaths{stroke, gaps, clipGaps(gaps, u.plot)}
}

// The adapter's series options: stroke colour and width.
const (
	seriesStroke = "#172839"
	seriesWidth  = 1.0
)

// drawPath for a line series, with strokeFill's clips and setCtxStyle's defaults.
func drawPath(ctx *js.Object, p plot, paths seriesPaths) {
	width := roundDec(seriesWidth*p.pxRatio, 3)
	// math.Mod is JS %.
	offset := math.Mod(width, 2) / 2
	pxAlign := offset > 0

	if pxAlign {
		ctx.Call("translate", offset, offset)
	}

	boundsClip := newPath2D()
	boundsClip.Call("rect", p.left-width/2, p.top-width/2, p.width+width, p.height+width)

	ctx.Set("strokeStyle", seriesStroke)
	ctx.Set("lineWidth", width)
	ctx.Set("lineJoin", "round")
	ctx.Set("lineCap", "butt")
	ctx.Call("save")
	ctx.Call("clip", boundsClip)
	if paths.clip != nil {
		ctx.Call("clip", paths.clip)
	}
	if width > 0 {
		ctx.Call("stroke", paths.stroke.path)
	}
	ctx.Call("restore")

	if pxAlign {
		ctx.Call("translate", -offset, -offset)
	}
}
