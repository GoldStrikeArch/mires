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
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'uplot',cases})+'\n')
