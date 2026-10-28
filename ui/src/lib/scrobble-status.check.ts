// Real watcher behavior with only the Tauri event/command boundary isolated.
const nodeModule: string = 'node:module';
const { registerHooks } = await import(nodeModule);
type Provider = 'lastfm' | 'listenbrainz';
type Status = { connected: boolean; username?: string | null; pending: number; failed: number };
type Counts = { pending: number; failed: number };
type Outbox = Record<Provider, Counts>;
const requests: Record<Provider, Array<(status: Status) => void>> = { lastfm: [], listenbrainz: [] };
const states = new Map<string, (state: Status) => void>();
const queues = new Set<(state: Outbox) => void>();
let notices = 0;
const harness = {
	status: (provider: Provider) => new Promise<Status>((resolve) => requests[provider].push(resolve)),
	state: (provider: string, cb: (state: Status) => void) => {
		states.set(provider, cb);
		return Promise.resolve(() => states.delete(provider));
	},
	queue: (cb: (state: Outbox) => void) => {
		queues.add(cb);
		return Promise.resolve(() => queues.delete(cb));
	},
	toast: Object.assign(() => { notices++; }, {
		success: () => { notices++; }, error: () => { notices++; }
	})
};
Object.defineProperties(globalThis, {
	$state: { value: (value: unknown) => value, configurable: true },
	__scrobbleHarness: { value: harness, configurable: true }
});
const modules = [new URL('./lastfm.svelte.ts', import.meta.url).href,
	new URL('./listenbrainz.svelte.ts', import.meta.url).href];
registerHooks({
	resolve(specifier: string, context: { parentURL?: string },
		next: (specifier: string, context: { parentURL?: string }) => unknown) {
		if (modules.includes(context.parentURL ?? '')) {
			const code = specifier === '$lib/api' ? `const h=globalThis.__scrobbleHarness;
				export const lastfmStatus=()=>h.status('lastfm'), listenbrainzStatus=()=>h.status('listenbrainz');
				export const onLastfmState=cb=>h.state('lastfm',cb), onListenBrainzState=cb=>h.state('listenbrainz',cb);
				export const onScrobbleOutboxChanged=cb=>h.queue(cb);
				export const onListenBrainzProblem=()=>Promise.resolve(()=>{});`
				: specifier === '$lib/player.svelte' ? 'export const toast=globalThis.__scrobbleHarness.toast;'
				: specifier === '$lib/i18n.svelte' ? 'export const t=key=>key;' : null;
			if (code) return { url: `data:text/javascript,${encodeURIComponent(code)}`, shortCircuit: true };
		}
		return next(specifier, context);
	}
});
function equal(actual: unknown, expected: unknown, message: string) {
	if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
}
async function settle() { for (let i = 0; i < 5; i++) await Promise.resolve(); }
const { lastfm, watchLastfm } = await import('./lastfm.svelte.ts');
const { listenbrainz, watchListenBrainz } = await import('./listenbrainz.svelte.ts');
const stops = [watchLastfm(), watchListenBrainz()];
await settle();
const outbox = { lastfm: { pending: 2, failed: 1 }, listenbrainz: { pending: 3, failed: 4 } };
for (const cb of queues) cb(outbox);
for (const provider of ['lastfm', 'listenbrainz'] as const) {
	equal(requests[provider].length, 1, 'Queue events use their payload without fetching status again');
	requests[provider].shift()!({ connected: true, username: provider, pending: 0, failed: 0 });
}
await settle();
equal(lastfm.connected, true, 'Initial account state still hydrates after a queue event');
equal(listenbrainz.connected, true, 'ListenBrainz account hydrates');
equal(lastfm.pending, 2, 'Newer Last.fm queue event wins over stale initial snapshot');
equal(lastfm.failed, 1, 'Last.fm failed count is visible');
equal(listenbrainz.pending, 3, 'ListenBrainz pending count follows its own queue');
equal(listenbrainz.failed, 4, 'ListenBrainz failed count is visible');
equal(notices, 0, 'Background queue/status changes produce no auth toasts');

states.get('lastfm')!({ connected: false, username: null, pending: 0, failed: 0 });
await settle();
equal(lastfm.connected, false, 'Disconnect follows the auth event');
states.get('lastfm')!({ connected: true, username: 'replacement', pending: 0, failed: 0 });
await settle();
const stale = requests.lastfm.shift()!;
const current = requests.lastfm.shift()!;
current({ connected: true, username: 'replacement', pending: 5, failed: 0 });
await settle();
stale({ connected: false, username: null, pending: 9, failed: 9 });
await settle();
equal(lastfm.connected, true, 'An older status response cannot undo a newer auth event');
equal(lastfm.pending, 5, 'An older account response cannot replace current queue counts');
states.get('lastfm')!({ connected: false, username: null, pending: 0, failed: 0 });
await settle();
const afterAuth = notices;
for (const cb of queues) cb({ lastfm: { pending: 0, failed: 0 }, listenbrainz: { pending: 1, failed: 0 } });
equal(lastfm.pending, 0, 'Cleared queue is reflected immediately');
equal(listenbrainz.failed, 0, 'Recovery clears failed count without reconnect');
equal(notices, afterAuth, 'Queue recovery does not announce a new connection');
stops.forEach((stop) => stop());
await settle();
equal(queues.size, 0, 'Teardown removes both queue listeners');
equal(states.size, 0, 'Teardown removes auth listeners');
for (const resolve of requests.lastfm) resolve({ connected: true, username: 'stale', pending: 9, failed: 9 });
await settle();
equal(lastfm.connected, false, 'An old request cannot resurrect a disconnected/disposed watcher');
console.log('scrobble-status.check: ok');
