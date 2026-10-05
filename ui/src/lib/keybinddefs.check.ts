// Self-check for the shortcut definitions (`keybinddefs.ts`). Same deal as `queue.check.ts`: no
// test runner in `ui/`, node 22 runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/keybinddefs.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. What this guards is the thing a
// typo breaks silently: two actions sharing one combo would make `actionFor` return whichever comes
// first, so the second action could never fire and the list would still look right.
import { KEYBIND_ACTIONS, sanitizeOverrides } from './keybinddefs.ts';
import { normalizeCombo, parseCombo } from './keycombo.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const ids = new Set<string>();
const combos = new Map<string, string>();

for (const a of KEYBIND_ACTIONS) {
	ok(!ids.has(a.id), `action id "${a.id}" is unique`);
	ids.add(a.id);
	ok(a.defaults.length > 0, `"${a.id}" has at least one default`);
	for (const combo of a.defaults) {
		const { key } = parseCombo(combo);
		ok(key.length > 0, `"${a.id}" default "${combo}" has a key`);
		const norm = normalizeCombo(combo);
		ok(norm.length > 0, `"${a.id}" default "${combo}" is not empty`);
		const owner = combos.get(norm);
		ok(owner === undefined, `default "${combo}" is not shared by "${a.id}" and "${owner}"`);
		combos.set(norm, a.id);
	}
}

// The two that have always answered to two keys each, and the one that has to differ per platform.
const playPause = KEYBIND_ACTIONS.find((a) => a.id === 'play_pause');
ok(!!playPause && playPause.defaults.length === 2, 'play/pause keeps Space and `;`');

// --- persisted overrides are validated on load --------------------------------------------------
// localStorage is user-writable; a hand-typed value must not leave an action on a combo the
// dispatcher drops, nor shadow the action that really owns one. F9/F10 are free (only F5 is a
// default) and `Shift+K` is unbindable, so these hold on any platform.
const keeps = (raw: unknown) => JSON.stringify(sanitizeOverrides(raw));
ok(keeps({ next: ['F9'] }) === '{"next":["F9"]}', 'a valid override survives');
ok(keeps(null) === '{}', 'a null blob yields no overrides');
ok(keeps({ nope: ['F9'] }) === '{}', 'an unknown action is dropped');
ok(keeps({ next: 'F9' }) === '{}', 'a non-list value is dropped');
ok(keeps({ next: ['F9', 7, 'Shift+K'] }) === '{"next":["F9"]}', 'unbindable and non-string entries are dropped');
ok(keeps({ play_pause: ['Shift+K'] }) === '{}', 'an override that is wholly unbindable leaves the default');
ok(keeps({ next: ['F9', 'F9'] }) === '{"next":["F9"]}', 'a duplicate within one action is collapsed');
ok(keeps({ next: [''] }) === '{}', 'an empty combo is dropped');
ok(keeps({ next: ['F5'] }) === '{}', 'an override colliding with another action\u2019s default is dropped');
// Two overrides that collide can only come from a hand-edit; whichever the pass reaches first is
// the one dropped, but exactly one survives and the set stays collision-free.
ok(
	keeps({ next: ['F9'], prev: ['F9'] }) === '{"prev":["F9"]}',
	'a clash between two overrides leaves exactly one'
);
// Reachable through the UI: rebind refresh off its F5 first, then give next the freed F5. The freed
// default must not be compared against, or next's binding is thrown away on the next launch.
ok(
	keeps({ refresh: ['F9'], next: ['F5'] }) === '{"next":["F5"],"refresh":["F9"]}',
	'a combo freed by another override is available to take'
);

console.log('ok');
