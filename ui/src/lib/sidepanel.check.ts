// Self-check for side panel width clamping, storage parsing, and drag calculation:
//
//     node --experimental-strip-types ui/src/lib/sidepanel.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app.
import {
	SIDE_PANEL_DEFAULT,
	SIDE_PANEL_MAX,
	SIDE_PANEL_MIN,
	clampPanelWidth,
	dragPanelWidth,
	parsePanelWidth
} from './sidepanel.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

ok(clampPanelWidth(400.6) === 401, 'rounds in-range width');
ok(clampPanelWidth(100) === SIDE_PANEL_MIN, 'clamps below minimum');
ok(clampPanelWidth(900) === SIDE_PANEL_MAX, 'clamps above maximum');
ok(clampPanelWidth(Number.NaN) === SIDE_PANEL_DEFAULT, 'NaN falls back to default');

ok(parsePanelWidth(null) === SIDE_PANEL_DEFAULT, 'null returns default');
ok(parsePanelWidth('') === SIDE_PANEL_DEFAULT, 'empty string returns default');
ok(parsePanelWidth('not-a-number') === SIDE_PANEL_DEFAULT, 'junk returns default');
ok(parsePanelWidth('440') === 440, 'valid stored width is preserved');
ok(parsePanelWidth('999') === SIDE_PANEL_MAX, 'stored width above max is clamped');

ok(dragPanelWidth(320, 500, 420) === 400, 'dragging left widens a right-docked panel');
ok(dragPanelWidth(320, 500, 540) === 280, 'dragging right narrows the panel');
ok(dragPanelWidth(320, 500, 50) === SIDE_PANEL_MAX, 'dragging far left clamps to max');
ok(dragPanelWidth(320, 500, 900) === SIDE_PANEL_MIN, 'dragging far right clamps to min');

console.log('ok');
