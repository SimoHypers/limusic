// Self-check for `lyricvoices.ts`: `node --experimental-strip-types ui/src/lib/lyricvoices.check.ts`
import { voices } from './lyricvoices.ts';

function ok(cond: boolean, what: string) {
	if (!cond) throw new Error(`broken: ${what}`);
}

const w = (text: string, bg = false) => ({ text, start_ms: 0, end_ms: 1, ...(bg ? { bg } : {}) });
const line = (agent?: string, words = [w('la')]) => ({ text: 'la', time_ms: 0, words, agent });
const far = (agents: (string | undefined)[]) => voices(agents.map((a) => line(a)))?.map((v) => v.far);

ok(voices([line(), line()]) === null, 'no singers, no background: nothing to draw');
ok(voices([line('v1'), line('v1'), line('v1000')]) === null, 'one singer is not a duet');
ok(voices([line('v2'), line('v2')]) === null, 'one singer, whatever their number');
ok(far(['v1', 'v2', 'v1'])!.join() === 'false,true,false', 'the second singer is on the far side');
ok(far(['v2', 'v1'])!.join() === 'true,false', 'by number, not by who sings first');
ok(far(['v1', 'v2', 'v1000', undefined])!.join() === 'false,true,false,false', 'everyone and unnamed stay near');
ok(far(['v1', 'v2', 'v3', 'v4'])!.join() === 'false,true,false,true', 'more singers take sides in turn');
ok(far(['v2', 'v5'])!.join() === 'false,true', 'the lowest number is near');
ok(far(['v1', 'singer', 'v2'])!.join() === 'false,false,true', 'an id that is not a singer stays near');

const mixed = voices([line(undefined, [w('one '), w('two '), w('(oh) ', true), w('(oh)', true), w('three')])])!;
ok(mixed[0].far === false, 'background words alone move nothing');
ok(mixed[0].runs!.map((r) => `${r.bg}:${r.words.length}`).join() === 'false:2,true:2,false:1', 'runs split where the voices meet');
const whole = voices([line('v1', [w('(ah)', true)]), line('v1')])!;
ok(whole[0].runs!.length === 1 && whole[0].runs![0].bg, 'a line that is all background is one run');
ok(whole[1].runs === null, 'a line without background words is drawn whole');
ok(voices([{ text: 'no words', time_ms: 0 }]) === null, 'a line with no words has none');
console.log('ok');
