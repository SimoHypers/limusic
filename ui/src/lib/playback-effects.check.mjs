// Exercise the actual command and startup callbacks with controlled IPC completion order.
// This Tauri/Svelte module cannot be imported directly in Node, so evaluate those two source
// sections with state and IPC stubs; no copies of their update logic live in this check.
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { runInNewContext } from 'node:vm';
import { presetSettings, sameEffects } from './effect-presets.ts';

function ok(condition, message) {
	if (!condition) throw new Error(message);
}
function deferred() {
	let resolve;
	let reject;
	const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
	return { promise, resolve, reject };
}
const source = readFileSync(new URL('./player.svelte.ts', import.meta.url), 'utf8');
const commandStart = source.indexOf('export function setTempoPitch(');
const commandEnd = source.indexOf('// playback-effects-check:end', commandStart);
const initStart = source.indexOf('export function initApp(');
ok(commandStart >= 0 && commandEnd > commandStart && initStart >= 0, 'Effect handlers must be found');
const code = stripTypeScriptTypes(
	source.slice(commandStart, commandEnd) + '\nlet started = false;\n' + source.slice(initStart),
	{ mode: 'strip' }
).replaceAll('export function ', 'function ');

function harness() {
	const snapshot = deferred();
	const requests = [];
	const callbacks = {};
	const errors = [];
	const playback = { speed: 1, semitones: 0, reverb: { wet: 25 } };
	const api = new Proxy({
		getPlayback: () => snapshot.promise,
		getQueue: () => Promise.resolve({ items: [] }),
		setPlaybackParams: (speed, semitones, reverb) => {
			const result = deferred();
			requests.push({ speed, semitones, reverb, result });
			return result.promise;
		}
	}, {
		get(target, key) {
			return target[key] ?? ((callback) => {
				callbacks[key] = callback;
				return Promise.resolve(() => {});
			});
		}
	});
	const context = { api, playback, toast: { error: (e) => errors.push(e) },
		forgetLocal() {}, applyLtState() {}, performance };
	const handlers = runInNewContext(code + '\n({ setTempoPitch, initApp })', context);
	handlers.initApp(true);
	return { ...handlers, playback, snapshot, requests, callbacks, errors };
}

const staleSnapshot = harness();
staleSnapshot.callbacks.onPlaybackParams({ speed: 0.8, semitones: 3, reverb: { wet: 40 } });
staleSnapshot.snapshot.resolve({ speed: 1, semitones: 0, reverb: { wet: 25 } });
await Promise.resolve();
ok(staleSnapshot.playback.speed === 0.8 && staleSnapshot.playback.reverb.wet === 40,
	'An older startup snapshot cannot replace another window\'s effects event');

const roomReset = harness();
const pending = roomReset.setTempoPitch(0.8, 3);
roomReset.callbacks.onPlaybackParams({ speed: 1, semitones: 3, reverb: { wet: 25 } });
roomReset.requests[0].result.resolve(undefined);
await pending;
ok(roomReset.playback.speed === 1 && roomReset.playback.semitones === 3,
	'A late command confirmation cannot overwrite the backend room reset');

const local = harness();
const first = local.setTempoPitch(0.8, -2, { wet: 40 });
ok(local.requests[0].speed === 0.8 && local.requests[0].semitones === -2 &&
	local.requests[0].reverb.wet === 40, 'Effects are sent directly to the backend');
local.requests[0].result.resolve(undefined);
await first;
ok(local.playback.speed === 1 && local.playback.semitones === 0,
	'Only confirmed backend events change the effects');
local.callbacks.onPlaybackParams({ speed: 0.8, semitones: -2, reverb: { wet: 40 } });
ok(local.playback.speed === 0.8 && local.playback.semitones === -2 && local.playback.reverb.wet === 40,
	'Confirmed effects are mirrored into playback state');

const concurrent = harness();
const a = concurrent.setTempoPitch(0.8, 0);
const b = concurrent.setTempoPitch(1.2, 0);
ok(concurrent.requests.length === 2, 'The backend owns command serialization');
concurrent.callbacks.onPlaybackParams({ speed: 0.8, semitones: 0, reverb: { wet: 25 } });
concurrent.callbacks.onPlaybackParams({ speed: 1.2, semitones: 0, reverb: { wet: 25 } });
concurrent.requests[1].result.resolve(undefined);
await b;
concurrent.requests[0].result.resolve(undefined);
await a;
ok(concurrent.playback.speed === 1.2, 'Reply order cannot replace the latest confirmed event');

const rejected = harness();
const failed = rejected.setTempoPitch(0.8, 0);
rejected.snapshot.resolve({ speed: 1.2, semitones: 0, reverb: { wet: 25 } });
await Promise.resolve();
ok(rejected.playback.speed === 1.2, 'The startup snapshot does not wait for a pending command');
const confirmedReverb = rejected.playback.reverb;
rejected.requests[0].result.reject('unavailable');
await failed;
ok(rejected.playback.speed === 1.2 && rejected.playback.reverb.wet === 25 &&
	rejected.playback.reverb !== confirmedReverb && rejected.errors[0] === 'unavailable',
	'Rejected effects retain confirmed values, reset slider drafts and surface the error');
const dialogSource = readFileSync(new URL('./components/TempoPitchDialog.svelte', import.meta.url), 'utf8');
const selectSource = dialogSource.match(/\t(?:async )?function selectPreset\([\s\S]*?\n\t}/)?.[0];
ok(selectSource, 'Preset selection handler must be found');
const preset = { name: 'Slow room', speed: 0.8, semitones: -2, reverb: { wet: 40 } };
const selection = harness();
const dialog = { presets: [preset], selectedName: '', apply: selection.setTempoPitch,
	presetSettings, sameEffects, playback: selection.playback, inRoom: () => false };
const selectPreset = runInNewContext(
	stripTypeScriptTypes(selectSource, { mode: 'strip' }) + '\nselectPreset', dialog);
const selected = selectPreset(preset.name);
selection.requests[0].result.resolve(undefined);
await selected;
ok(dialog.selectedName === preset.name && !sameEffects(preset, selection.playback),
	'Preset selection survives a reply arriving before its confirmation event');
selection.callbacks.onPlaybackParams(preset);
ok(sameEffects(preset, selection.playback), 'The event confirms the selected sound');
console.log('ok — backend-confirmed effects and startup snapshot');
