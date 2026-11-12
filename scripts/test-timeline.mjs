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
// One line per game: stretches end to end, breaks collapsed, positions stable as footage comes and goes.
const close = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} vs ${b}`);
import { buildLine, toReal, toLine, footage, wallClock } from '../app/src/lib/gameline.ts';
const day = [{ id: 'a', created_ms: 1000, intervals: [[0, 100], [110, 200]] }, { id: 'b', created_ms: 9_000_000, intervals: [[0, 50]] }];
const line = buildLine(day);
assert.equal(line.total, 240);
assert.equal(line.intervals.length, 3);
close(line.intervals[1][0] - line.intervals[0][1], line.gap); assert.ok(line.gap > 0, 'a break between stretches');
assert.equal(toReal(line, 100 + line.gap + 5).session, 'a'); close(toReal(line, 100 + line.gap + 5).t, 115);
assert.equal(toReal(line, 100 + line.gap / 2), null, 'nothing in a break');
close(toLine(line, 'b', 10), 190 + 2 * line.gap + 10);
assert.equal(toLine(line, 'a', 105, .3), null, 'a moment that was not recorded has no place');
close(footage(line, toLine(line, 'b', 10)), 200);
close(wallClock(line, toLine(line, 'b', 10)), 9_010_000);
for (let v = 0; v < line.intervals.at(-1)[1]; v += 7.3) { const r = toReal(line, v); if (r) assert.ok(Math.abs(toLine(line, r.session, r.t) - v) < 1e-9, 'round trip'); }
// The oldest 30 s age out and 20 s are recorded at the end: every surviving moment keeps its place.
const later = buildLine([{ ...day[0], intervals: [[30, 100], [110, 200]] }, { ...day[1], intervals: [[0, 70]] }], line);
assert.equal(later.gap, line.gap);
close(toLine(later, 'a', 60), toLine(line, 'a', 60));
close(toLine(later, 'b', 10), toLine(line, 'b', 10));
close(later.intervals[0][0], 30);
// A whole early stretch is gone: still stable.
const pruned = buildLine([{ ...day[0], intervals: [[150, 200]] }, { ...day[1], intervals: [[0, 70]] }], later);
close(toLine(pruned, 'a', 160), toLine(line, 'a', 160));
console.log('PASS timeline and game-line model: 5 duration scales, cursor invariance, bounded panning, gaps, original timestamps, 1000-event clustering');
