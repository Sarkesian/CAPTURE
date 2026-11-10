import assert from 'node:assert/strict';
import { zoomView, fitView, tickStep, eventGroups, selectionAround, validSelection, clock, parseClock } from '../app/src/lib/timeline.ts';
for(const duration of [120,1800,7200,14400,21600]) {
  let v={a:0,b:duration};const cursor=duration*.35;
  for(let i=0;i<14;i++) {
    const previous=v,ratio=(cursor-v.a)/(v.b-v.a);v=zoomView(v,.5,cursor,0,duration);
    assert.ok(v.b-v.a>=2);assert.ok(Math.abs(v.a+(v.b-v.a)*ratio-cursor)<.00001);
    assert.ok(tickStep(v.b-v.a,700)>0);
  }
  v=fitView(-100,60,0,duration);assert.equal(v.a,0);
  v=fitView(duration+10,60,0,duration);assert.equal(v.b,duration);
}
const intervals=[[100,300],[360,600]];
assert.deepEqual(selectionAround(intervals,110,30,15),{a:100,b:125});
assert.equal(selectionAround(intervals,330,30,15),null);
assert.equal(validSelection(intervals,{a:280,b:380}),false);
assert.equal(validSelection(intervals,{a:100,b:300}),true);
assert.equal(clock(14406.25,true),'4:00:06.3');
assert.equal(clock(21552.1,true),'5:59:12.1');
assert.equal(parseClock('5:59:12.1'),21552.1);
assert.equal(parseClock('1:70:00'),null);
assert.equal(parseClock('invalid'),null);
const events=Array.from({length:1000},(_,i)=>({id:String(i),t:7200+i*.001,kind:'marker'}));
const groups=eventGroups(events,{a:0,b:21600},800);
assert.equal(groups.length,1);assert.equal(groups[0].events.length,1000);
console.log('PASS timeline model: 5 duration scales, cursor invariance, bounded panning, gaps, original timestamps, 1000-event clustering');
