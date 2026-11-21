import {writeFileSync} from 'node:fs'
const cases = []
const add = (id, spanGaps, ...frames) => cases.push({id, input:{spanGaps,frames}})
function frame(xs, ys, size=[64,40], xRange=[xs[0],xs.at(-1)], yRange=[-8,8]) {
  return {size,xRange,yRange,data:[xs,...ys],probes:[
    [xRange[0],yRange[1],0,0], [xRange[1],yRange[0],...size],
    [(xs.length > 1 ? xs[0]+xs[1] : xRange[0]+xRange[1])/2,1.25,size[0]/2,size[1]/3],
    [xRange[0]-2,yRange[1]+3,-5,size[1]+10]
  ]}
}
const x = [0,1,2,4,7,8,12]
const y = [null,2,null,null,-3,5,null]
add('sparse',false,frame(x,[[0,1,4,-1,6,-4,3]]))
add('multiple',false,frame(x,[y,[1,null,2,null,4,null,8]]))
add('span-gaps',true,frame(x,[y]))
add('all-null',false,frame(x,[x.map(()=>null)]))
add('one-value',false,frame(x,[[null,null,null,-1,null,null,null]]))
add('repeated-x',false,frame([-3,-2,-2,0,1,1,1,2],[[0,4,-2,1,null,3,5,2]]))
add('resize-and-replace',false,frame(x,[y]),frame([0,2,3,7],[[0,2,-4,1]],[89,57]),frame([5,9],[[1,1]],[31,25],[3,11],[-2,2]))
function dense(count,size) {
  const xs = Array.from({length:count},(_,i)=>i)
  return frame(xs,[xs.map(i=>i%31===4 || i%31===5 ? null : (i*23)%17-8),
    xs.map(i=>i%57===0 ? null : Math.floor(i/7)%13-6)],size)
}
for (const count of [64,65,66,137,331]) add('density-'+count,false,dense(count,[16,24]))
const empty = frame([], [[]], [95.5,55.5], [0,12])
const filled = frame(x, [y], [95.5,55.5])
add('empty-with-probes',false,empty)
add('empty-without-probes',true,{...empty,probes:[]})
add('populated-to-empty',false,filled,empty)
add('populated-empty-populated',true,filled,empty,filled)
add('empty-two-series',false,frame([], [[],[]], [64.25,40.75], [0,12]))
add('fractional-dense',false,dense(331,[95.2,55.78]))
add('bench-lines',false,dense(4096,[256,144]))

// The 2026-11-20 review (R-uplot): its failing inputs, its other hand-built cases, and adversarial
// neighbours. Each frame lists its own probes.
const lin = n => Array.from({length:n}, (_, i) => i)
const plain = (size, xRange, yRange, data, probes=[]) => ({size, xRange, yRange, data, probes})
// calcSize's incrRound(v, 0.5) runs fixFloat first: 64.49999998 becomes 64.5, then 65 half-pixels.
add('fixfloat-plotdim',false,plain([32.24999999,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[4,0,0,0]]))
add('fixfloat-plotdim-y',false,plain([32,20.24999999],[0,4],[0,4],[lin(5),[0,1,2,3,4]]))
// fixFloat's /\.\d*?(?=9{6,}|0{6,})/: six 9s snap and five do not, a run after other digits, a run of 0s.
add('fixfloat-six-nines',false,plain([32.24999995,41.24999999],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[4,0,32,41]]))
add('fixfloat-five-nines',false,plain([32.2499995,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[4,0,32,20]]))
add('fixfloat-later-run',false,plain([47.7499999999,18.7499999995],[0,8],[0,8],[lin(9),[8,0,7,1,6,2,5,3,4]],[[8,8,47,18]]))
add('fixfloat-zeros',false,plain([16.0500000001,25.2000000004],[0,4],[0,4],[lin(5),[4,3,null,1,0]],[[2,2,8,12]]))
// JS Math.round, not floor(v + 0.5): 0.49999999999999994 rounds to 0, and precise() of an odd
// integer in [2^52, 2^53) keeps it.
add('round-just-below-half',false,plain([64,40],[0,64],[0,8],[[0,0.49999999999999994,64],[1,2,3]]))
add('round-odd-2p52',false,plain([64,40],[4503599627.370497,4503599628.370497],[0,8],
  [[4503599627.370497,4503599628.370497],[1,2]],[[4503599627.370497,1,0,0]]))
// -0, which JSON.parse keeps: getHPos adds the plot offset, so a -0 x pixel becomes 0.
add('negative-zero-x',false,plain([64,40],[0,2],[0,8],[[-0,1,2],[1,null,3]]))
add('negative-zero-probes',true,plain([64,40],[-0,2],[-0,8],[[-0,0,1,2],[-0,0,-0,8]],[[-0,-0,-0,-0],[0,0,0,0]]))
// A decimated column's minimum and maximum, where -0 and 0 compare equal.
add('negative-zero-dense-y',false,plain([16,24],[0,79],[-4,4],[lin(80),lin(80).map(i=>[-0,0,3,-3][i%4])],[[39,-0,8,12]]))
// One point, repeated x values only, and a visible window whose upper index is 0 (closestIdx's
// `hi || arr.length - 1`).
add('single-point',false,plain([10,10],[0,1],[0,1],[[0.5],[0.5]],[[0.5,0.5,5,5]]))
add('all-same-x',false,plain([10,10],[0,1],[0,4],[lin(41).map(()=>0.5),lin(41).map(i=>i%5)],[[0.5,1,3,3]]))
add('window-hi-zero',false,plain([40,20],[-10,0.2],[0,4],[[0,1,2,3,4],[1,2,3,2,1]],[[0,1,39,3],[3,1,0,3]]))
// Coordinates, vertices and probes past int32 and past 2^53.
add('past-int32',false,plain([64,40],[0,1],[0,1],[[-5e7,0,0.5,1,4e7,4e7+0.25],[1,3e9,null,-3e9,0.5,2**31]],
  [[4e7,3e9,2**31+0.5,-(2**31)-0.5],[-5e7,-3e9,3e9,3e9]]))
add('past-2p53',false,plain([64,40],[2**53,2**53+2**20],[2**53,2**53+2**10],
  [[2**53,2**53+2**18,2**53+2**19,2**53+2**20,2**54+2],[2**53+2,null,2**53+2**9,2**53+2**10,2**54]],
  [[2**53+2**10,2**53+6,32,20],[2**54,2**54,2**53,2**53]]))
// Wide and long: 32 series over 48 shared x values, and 12,000 decimated samples in one series.
const wideX = lin(48).map(i => i*1.5)
add('wide-series',false,plain([96,64],[0,70.5],[-10,10],
  [wideX,...lin(32).map(s => wideX.map((_, i) => (i+s)%11 === 0 ? null : (i*7+s*3)%21-10))],[[35,0,48,32]]))
const longX = lin(12000).map(i => Math.floor(i/3))
add('long-dense',false,plain([200,100],[0,longX.at(-1)],[-10,10],
  [longX,longX.map((_, i) => i%97 < 3 ? null : (i*37)%19-9)],[[1000,0,100,50]]))
// devicePixelRatio round trips (CONTRACT.md): at 1.75, 1.1 and 2.625 these sizes come back from
// size / pxRatio * pxRatio just below the half pixel, so the canvas rounds down, and at 1.75 the
// probe at pixel 3 differs from 3 / 122.88 in the last place. check.mjs runs at ratio 1.
add('dpr-canvas-roundtrip',false,
  plain([30.5,57.5],[0,7],[0,8],[[0,2,3,7],[1,4,null,6]],[[3,4,15,28]]),
  plain([14.5,20.5],[0,7],[0,8],[[0,2,3,7],[1,4,null,6]],[[3,4,7,10]]),
  plain([25.5,29.5],[0,7],[0,8],[[0,2,3,7],[1,4,null,6]],[[3,4,12,14]]))
add('dpr-probe-roundtrip',false,plain([122.88,40],[0,1000],[0,1000],[[0,250,500,1000],[0,750,null,1000]],[[500,500,3,3]]))

// Re-review (re-uplot) regressions: overflowing finite scale bounds (NaN pixels in dense
// decimation and gap scans) and uPlot's < 1e-16 setScale range guard across frames.
add('overflow-range',false,plain([64,40],[-1e308,1e308],[-1.5e308,1.5e308],
  [[-1e308,0,1e308],[1e308,null,-1e308]],[[0,0,32,20],[1e308,-1e308,-1,41]]))
const ox = lin(300).map(i => 9e307 + i * 2e304)
add('dense-overflow-nan',false,plain([16,24],[-1e308,1e308],[-8,8],
  [ox,ox.map((_, i) => i % 7 === 3 ? null : (i * 5) % 17 - 8)],[[1e308,0,8,12]]))
add('dense-overflow-nan-span',true,plain([16,24],[-1e308,1e308],[-8,8],
  [ox,ox.map((_, i) => i % 7 === 3 ? null : (i * 5) % 17 - 8)],[[1e308,0,8,12]]))
const oy = lin(300).map(i => -1e308 + i * 6e305)
add('dense-overflow-mixed',false,plain([16,24],[-1e308,1e308],[-8,8],
  [oy,oy.map((_, i) => (i * 5) % 17 - 8)],[[0,0,8,12]]))
add('tiny-range',false,
  plain([40,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[2,2,20,10]]),
  plain([40,20],[0,1e-17],[0,4],[[0,5e-18,1e-17],[1,2,3]],[[5e-18,2,20,10]]))
add('tiny-range-recover',false,
  plain([40,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[2,2,20,10]]),
  plain([40,20],[0,1e-17],[0,4],[[0,5e-18,1e-17],[1,2,3]],[[5e-18,2,20,10]]),
  plain([40,20],[0,10],[0,8],[[0,2,5,10],[0,4,null,8]],[[5,4,20,10]]))
add('tiny-range-y',false,
  plain([40,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[2,2,20,10]]),
  plain([40,20],[0,4],[0,5e-17],[lin(5),[0,1e-17,2e-17,3e-17,5e-17]],[[2,1e-17,20,10]]))
add('tiny-range-wide-data',false,
  plain([40,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[2,2,20,10]]),
  plain([40,20],[0,1e-17],[0,4],[lin(5),[1,2,3,2,1]],[[5e-18,2,20,10]]))
add('tiny-data-normal-range',false,
  plain([40,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[2,2,20,10]]),
  plain([40,20],[0,4],[0,4],[[0,5e-18,1e-17],[1,2,3]],[[5e-18,2,20,10]]))
add('tiny-single-frame',false,
  plain([40,20],[0,1e-17],[0,4],[[0,5e-18,1e-17],[1,2,3]],[[5e-18,2,20,10]]))
add('tiny-single-frame-both',false,
  plain([40,20],[0,1e-17],[0,1e-17],[[0,5e-18,1e-17],[0,null,1e-17]],[[0,0,20,10],[5e-18,5e-18,-5,-5]]),
  plain([40,20],[0,1e-17],[0,4],[[0,5e-18,1e-17],[1,2,3]],[[5e-18,2,20,10]]),
  plain([40,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[2,2,20,10]]))
add('tiny-range-both-recover',false,
  plain([40,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[[2,2,20,10]]),
  plain([40,20],[0,1e-17],[0,1e-17],[[0,5e-18,1e-17],[0,5e-18,1e-17]],[[5e-18,5e-18,20,10]]),
  plain([40,20],[0,10],[0,8],[[0,2,5,10],[0,4,null,8]],[[5,4,20,10]]))
add('tiny-range-flat-x',false,
  plain([40,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[]),
  plain([40,20],[0,9e-17],[0,4],[[4.9500000499999994e-17,4.9500000499999994e-17,4.9500000499999994e-17],[1,2,3]],[[4.95e-17,2,20,10]]),
  plain([40,20],[0,9e-17],[0,4],[[4.95000005e-17,4.95000005e-17,4.95000005e-17],[1,2,3]],[[4.95e-17,2,20,10]]),
  plain([40,20],[0,1e-17],[0,4],[[0,0,0],[1,2,3]],[[5e-18,2,20,10]]))
add('tiny-range-exact-1e-16',false,
  plain([40,20],[0,4],[0,4],[lin(5),[0,1,2,3,4]],[]),
  plain([40,20],[0,9.999999999999999e-17],[0,4],[[0,9.999999999999999e-17],[1,2]],[[0,2,20,10]]),
  plain([40,20],[0,1e-16],[0,4],[[0,1e-16],[1,2]],[[0,2,20,10]]))

// JSON.stringify writes -0 as 0, so the negative-zero cases above pass through a marker.
const text = JSON.stringify({schema:1,library:'uplot',cases}, (_, value) => Object.is(value, -0) ? 'NEGATIVE_ZERO' : value)
writeFileSync(new URL('corpus.json',import.meta.url), text.replaceAll('"NEGATIVE_ZERO"', '-0')+'\n')
