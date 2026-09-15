// Persistent appearance state. Run from the repository root with Node 22.16+:
//   node ui/scripts/appearance.check.mjs
// Uses the installed Svelte compiler/runtime, with storage and theme-only DOM APIs mocked.
// --logic-only explicitly substitutes an identity $state for offline storage-logic checks.
// That mode does NOT validate Svelte reactivity, component mounting or native WebViews.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';

const logicOnly = process.argv.includes('--logic-only');
let compileModule;
if (!logicOnly) {
	({ compileModule } = await import('svelte/compiler'));
}

let source = stripTypeScriptTypes(readFileSync(new URL('../src/lib/theme.svelte.ts', import.meta.url), 'utf8'));
// Mock known imported bindings, not entire import lines. Upstream can remove/reorder a binding
// (such as nearestHue) without invalidating storage coverage. Unknown dependencies still fail.
const mocks = new Map([
	['@tauri-apps/api/core', { convertFileSrc: '(path) => path' }],
	['./color', { hexToHsv: '() => null', isLight: '() => false', nearestHue: '(_from, hue) => hue' }],
	['./artcolor', { artworkAccent: 'async () => null', toAccent: '(color) => color', warmAccent: '() => {}' }],
	['./api', { allowFontFile: 'async () => {}' }]
]);
const mocked = new Set();
source = source.replace(/^import\s*\{([^}]+)\}\s*from\s*['"]([^'"]+)['"];?/gm,
	(_line, bindings, module) => {
		const exports = mocks.get(module);
		assert(exports, `Add an explicit test mock for module: ${module}`);
		mocked.add(module);
		return bindings.split(',').map((binding) => {
			const [name, local = name] = binding.trim().split(/\s+as\s+/);
			assert(Object.hasOwn(exports, name), `Add an explicit test mock for ${module}: ${name}`);
			return `const ${local} = ${exports[name]};`;
		}).join('\n');
	});
assert.deepEqual([...mocked].sort(), [...mocks.keys()].sort(), 'Expected theme dependencies were mocked');
if (logicOnly) {
	source = `const $state = (initial) => initial;\n${source}`;
} else {
	source = compileModule(source, { filename: 'theme.svelte.js', generate: 'client' }).js.code;
	// A data URL cannot resolve a bare package import. Use this checkout's installed runtime.
	source = source.replace(/(['"])svelte\/internal\/client\1/g,
		JSON.stringify(import.meta.resolve('svelte/internal/client')));
}

let assertions = 0;
function equal(actual, expected, message) {
	assertions++;
	assert.deepEqual(actual, expected, message);
}
const originals = new Map(['localStorage', 'document', 'getComputedStyle'].map((key) =>
	[key, Object.getOwnPropertyDescriptor(globalThis, key)]));
function global(key, value) {
	Object.defineProperty(globalThis, key, { value, configurable: true, writable: true });
}
const style = { setProperty() {}, removeProperty() {}, getPropertyValue: () => '' };
global('document', {
	documentElement: { style, classList: { add() {}, remove() {}, contains: () => false } },
	createElement: () => ({ getContext: () => null })
});
global('getComputedStyle', () => style);

let serial = 0;
async function fresh(raw, { readFails = false } = {}) {
	const data = new Map();
	if (raw !== undefined) data.set('appearance', raw);
	let failWrites = false;
	let writes = 0;
	global('localStorage', {
		getItem(key) {
			if (readFails && key === 'appearance') throw new Error('Storage read denied');
			return data.get(key) ?? null;
		},
		setItem(key, value) {
			if (failWrites) throw new Error('Storage quota exceeded');
			data.set(key, value);
			writes++;
		}
	});
	const url = 'data:text/javascript;base64,' + Buffer.from(source + `\n// instance ${serial++}`).toString('base64');
	const mod = await import(url);
	mod.initTheme();
	return {
		...mod, data,
		get writes() { return writes; },
		failWrites: (fail) => { failWrites = fail; }
	};
}

try {
	let app = await fresh();
	equal(app.appearance.queueHistoryVisible, false, 'new installations hide history');
	equal(app.writes, 0, 'startup does not persist a default');

	for (const visible of [true, false]) {
		app = await fresh(JSON.stringify({ queueHistoryVisible: visible }));
		equal(app.appearance.queueHistoryVisible, visible, 'both persisted values hydrate');
		equal(app.writes, 0, 'hydration is read-only');
		app.setAppearance({ queueHistoryVisible: !visible });
		const persisted = app.data.get('appearance');
		equal(JSON.parse(persisted).queueHistoryVisible, !visible, 'explicit change persists');
		app = await fresh(persisted);
		equal(app.appearance.queueHistoryVisible, !visible, 'fresh module restores persisted value');
	}

	for (const raw of ['{', 'null', '0', 'true', '"true"', '[]',
		'{"queueHistoryVisible":"true"}', '{"queueHistoryVisible":1}',
		'{"queueHistoryVisible":null}', '{"queueHistoryVisible":{}}']) {
		app = await fresh(raw);
		equal(app.appearance.queueHistoryVisible, false, `reject malformed/nonboolean value: ${raw}`);
		equal(app.data.get('appearance'), raw, 'invalid data is not overwritten during hydration');
	}

	app = await fresh('{"artworkBackground":false,"tabbedPlayer":false,"openPlayerOnPlay":false,"artworkAccent":true}');
	equal(app.appearance.queueHistoryVisible, false, 'older appearance objects migrate without a write');
	app.setAppearance({ queueHistoryVisible: true });
	equal(JSON.parse(app.data.get('appearance')), {
		artworkBackground: false, tabbedPlayer: false, openPlayerOnPlay: false,
		artworkAccent: true, queueHistoryVisible: true
	}, 'saving history preserves the other appearance preferences');
	app.setAppearance({ tabbedPlayer: true });
	equal(JSON.parse(app.data.get('appearance')).queueHistoryVisible, true,
		'an unrelated appearance change preserves history visibility');

	app = await fresh('{"queueHistoryVisible":true,"unexpected":true,"__proto__":{"polluted":true}}');
	equal(app.appearance.queueHistoryVisible, true, 'known boolean still loads');
	equal(Object.hasOwn(app.appearance, 'unexpected'), false, 'unknown properties are not hydrated');
	equal(Object.hasOwn(app.appearance, 'polluted'), false, 'prototype-shaped input is not hydrated');

	app = await fresh('{"queueHistoryVisible":true}', { readFails: true });
	equal(app.appearance.queueHistoryVisible, false, 'appearance read failure keeps default');
	equal(app.writes, 0, 'read failure does not overwrite stored data');

	app = await fresh('{"queueHistoryVisible":false}');
	const oldData = app.data.get('appearance');
	app.failWrites(true);
	assert.throws(() => app.setAppearance({ queueHistoryVisible: true }), /Storage quota/);
	assertions++;
	equal(app.appearance.queueHistoryVisible, true, 'failed save keeps the shared in-session choice');
	equal(app.data.get('appearance'), oldData, 'failed write does not claim persistence');
	equal(app.writes, 0, 'failed write is not counted as saved');
	app.failWrites(false);
	app.setAppearance({ queueHistoryVisible: false });
	app.setAppearance({ queueHistoryVisible: true });
	equal(JSON.parse(app.data.get('appearance')).queueHistoryVisible, true, 'later toggles retry successfully');

	app = await fresh();
	app.data.set('custom-theme', '{"radius":0.7}');
	app.data.set('primary-theme', 'teal');
	for (let i = 0; i < 31; i++) {
		app.setAppearance({ queueHistoryVisible: !app.appearance.queueHistoryVisible });
	}
	equal(app.appearance.queueHistoryVisible, true, 'rapid changes use the current shared value');
	equal(JSON.parse(app.data.get('appearance')).queueHistoryVisible, true, 'last rapid change persists');
	equal(app.writes, 31, 'only explicit changes write');
	equal(app.data.get('custom-theme'), '{"radius":0.7}', 'custom theme storage untouched');
	equal(app.data.get('primary-theme'), 'teal', 'primary theme storage untouched');

	console.log(`appearance: ${assertions} assertions passed (${logicOnly ? 'storage logic only; $state mocked' : 'Svelte-compiled module; DOM/storage mocked'})`);
} finally {
	for (const [key, descriptor] of originals) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else delete globalThis[key];
	}
}
