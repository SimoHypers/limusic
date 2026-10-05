// Self-check for the shortcut combo vocabulary (`keycombo.ts`). Same deal as `queue.check.ts`: no
// test runner in `ui/`, node 22 runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/keycombo.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. What this guards is the part
// the editable bindings depend on: the string a recorded key event produces has to compare equal to
// the default it is checked against, whatever order the modifiers arrived in.
import { comboFromEvent, isLocalBindable, keyTokenFromCode, normalizeCombo, parseCombo } from './keycombo.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

/** Only the fields `comboFromEvent` reads. */
function evt(init: {
	code: string;
	key: string;
	ctrlKey?: boolean;
	altKey?: boolean;
	shiftKey?: boolean;
	metaKey?: boolean;
}): KeyboardEvent {
	return { ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...init } as KeyboardEvent;
}

// --- key tokens ------------------------------------------------------------------------------
ok(keyTokenFromCode('KeyF') === 'F', 'a letter comes out as itself');
ok(keyTokenFromCode('Digit0') === '0', 'a digit comes out as itself');
ok(keyTokenFromCode('Numpad5') === 'Numpad5', 'numpad keys keep their code');
ok(keyTokenFromCode('F5') === 'F5', 'function keys keep their code');
ok(keyTokenFromCode('Space') === 'Space', 'Space is named');
ok(keyTokenFromCode('Semicolon') === ';', 'punctuation maps to the printed key');
ok(keyTokenFromCode('IntlBackslash') === null, 'an unbindable key is refused');

// --- recording ------------------------------------------------------------------------------
ok(comboFromEvent(evt({ code: 'Space', key: ' ' })) === 'Space', 'a bare key is bindable locally');
ok(
	comboFromEvent(evt({ code: 'Space', key: ' ' }), true) === null,
	'a bare key is refused for a global grab'
);
ok(comboFromEvent(evt({ code: 'F5', key: 'F5' }), true) === 'F5', 'an F-key is fine globally');
ok(
	comboFromEvent(evt({ code: 'ShiftLeft', key: 'Shift' })) === null,
	'a modifier on its own is not a combo'
);
ok(
	comboFromEvent(evt({ code: 'KeyM', key: 'M', metaKey: true, shiftKey: true })) === 'Shift+Super+M',
	'modifiers come out in one fixed order'
);

// --- equivalence ----------------------------------------------------------------------------
// The whole point: a default written the way it reads and an event recorded the other way round
// have to match, or a shortcut would look bound but never fire.
ok(
	normalizeCombo('Super+Shift+M') === normalizeCombo('Shift+Super+M'),
	'modifier order does not matter'
);
ok(
	normalizeCombo(comboFromEvent(evt({ code: 'KeyM', key: 'M', metaKey: true, shiftKey: true }))!) ===
		normalizeCombo('Super+Shift+M'),
	'a recorded mute matches the default'
);
// Shift is layout noise on `>`, `<` and `+`: one binding answers to both forms.
ok(normalizeCombo('Super+Shift+.') === normalizeCombo('Super+.'), 'Shift+. is the same as .');
ok(normalizeCombo('Ctrl+Shift+=') === normalizeCombo('Ctrl+='), 'Shift+= is the same as =');
ok(normalizeCombo('Super+Shift+K') !== normalizeCombo('Super+K'), 'Shift+K is its own binding');
ok(normalizeCombo('Space') === 'space', 'a bare named key folds to its lowercase token');
ok(normalizeCombo('Space') === normalizeCombo('space'), 'a bare key is case-folded');

// --- parsing --------------------------------------------------------------------------------
const parsed = parseCombo('Super+Shift+M');
ok(parsed.key === 'M', 'the key is the last token');
ok(parsed.mods.join(',') === 'Super,Shift', 'the modifiers are everything before it');

// --- bindable shapes ------------------------------------------------------------------------
// The recorder refuses exactly what the dispatcher drops, so a saved row always fires.
ok(isLocalBindable('Super+K'), 'a primary-modifier combo is bindable');
ok(isLocalBindable('Super+Shift+K'), 'Shift alongside a primary modifier is bindable');
ok(isLocalBindable('Space'), 'a bare key is bindable (it just does not fire while typing)');
ok(isLocalBindable('F5'), 'a bare F-key is bindable');
ok(isLocalBindable('Shift+F5'), 'Shift with an F-key is bindable');
ok(isLocalBindable('Alt+K'), 'Alt on its own is bindable');
ok(isLocalBindable('Alt+Shift+K'), 'Alt+Shift is bindable');
ok(!isLocalBindable('Shift+K'), 'Shift with a bare letter is not');
ok(!isLocalBindable('Shift+Space'), 'Shift with a bare named key is not');
ok(!isLocalBindable(''), 'an empty combo is not');
ok(!isLocalBindable('Ctrl+Alt+P'), 'Ctrl+Alt belongs to the global hotkeys');
ok(!isLocalBindable('Super+Alt+P'), 'Super+Alt belongs to the global hotkeys');

console.log('ok');
