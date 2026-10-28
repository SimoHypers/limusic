// Exercise the installed window listener; isolate Tauri/Svelte imports, not shortcut behavior.
const nodeModule: string = 'node:module';
const { registerHooks } = await import(nodeModule);
function equal(actual: unknown, expected: unknown, message: string) {
	if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
}

class TestElement {
	tagName: string;
	parentElement: TestElement | null;
	private attrs: Record<string, string>;
	constructor(tagName: string, parentElement: TestElement | null = null, attrs: Record<string, string> = {}) {
		this.tagName = tagName;
		this.parentElement = parentElement;
		this.attrs = attrs;
	}
	getAttribute(name: string) { return this.attrs[name] ?? null; }
	closest(selector: string): TestElement | null {
		for (let node: TestElement | null = this; node; node = node.parentElement) {
			if (selector.split(',').some((part) => {
				const match = /^\[([^=\]]+)(?:=([^\]]+))?\]$/.exec(part);
				return match && node!.getAttribute(match[1]) !== null &&
					(match[2] === undefined || node!.getAttribute(match[1]) === match[2]);
			})) return node;
		}
		return null;
	}
}
class TestHTMLElement extends TestElement {
	isContentEditable: boolean;
	constructor(tag: string, parent: TestElement | null = null, attrs: Record<string, string> = {},
		isContentEditable = false) { super(tag, parent, attrs); this.isContentEditable = isContentEditable; }
	blur() { if (focused === this) focused = null; }
}
let handler: ((event: KeyboardEvent) => void) | undefined;
let focused: TestHTMLElement | null = null;
const listeners = new Map<string, (event: unknown) => void>();
let toggles = 0;
Object.defineProperties(globalThis, {
	Element: { value: TestElement, configurable: true },
	HTMLElement: { value: TestHTMLElement, configurable: true },
	document: { value: { get activeElement() { return focused; } }, configurable: true },
	window: { value: {
		addEventListener: (type: string, callback: (event: unknown) => void, capture = false) => {
			listeners.set(`${type}:${capture}`, callback);
			if (type === 'keydown' && !capture) handler = callback;
		},
		removeEventListener: (type: string, callback: (event: unknown) => void, capture = false) => {
			if (listeners.get(`${type}:${capture}`) === callback) listeners.delete(`${type}:${capture}`);
			if (type === 'keydown' && !capture) handler = undefined;
		}
	}, configurable: true },
	__shortcutToggle: { value: () => { toggles++; }, configurable: true }
});
const source = new URL('./shortcuts.ts', import.meta.url).href;
registerHooks({
	resolve(specifier: string, context: { parentURL?: string },
		next: (specifier: string, context: { parentURL?: string }) => unknown) {
		if (context.parentURL === source) {
			const code = specifier === '$app/environment' ? 'export const browser = false;'
				: specifier === './api' ? 'export const togglePause = () => globalThis.__shortcutToggle();'
				: specifier === './player.svelte' ? `export const cycleRepeat=()=>{}, np={}, nudgeVolume=()=>{},
				playback={}, refreshView=()=>{}, toggleMute=()=>{}, ui={};` : null;
			if (code) return { url: `data:text/javascript,${encodeURIComponent(code)}`, shortCircuit: true };
		}
		return next(specifier, context);
	}
});
const { initShortcuts } = await import('./shortcuts.ts');
const cleanup = initShortcuts();
function press(target: TestElement | null, key = ' ', prevented = false) {
	let consumed = prevented;
	handler!({ key, target, defaultPrevented: prevented, ctrlKey: false, metaKey: false,
		altKey: false, shiftKey: false, preventDefault: () => { consumed = true; } } as unknown as KeyboardEvent);
	return consumed;
}
for (const tag of ['BUTTON', 'INPUT', 'TEXTAREA', 'SELECT', 'A', 'SUMMARY', 'AUDIO', 'VIDEO']) {
	const control = new TestHTMLElement(tag, null, tag === 'A' ? { href: '#' } : {});
	equal(press(control), false, `${tag} keeps native Space`);
	equal(press(new TestElement('svg', control)), false, `${tag} nested child keeps Space`);
}
for (const role of ['button', 'checkbox', 'switch', 'radio', 'tab', 'menuitem', 'slider', 'combobox', 'listbox']) {
	const control = new TestHTMLElement('DIV', null, { role });
	equal(press(new TestHTMLElement('SPAN', control)), false, `${role} keeps Space`);
}
equal(press(new TestHTMLElement('SPAN', null, {}, true)), false, 'Contenteditable keeps Space');
equal(press(new TestHTMLElement('SPAN', new TestHTMLElement('DIV', null, {}, true))),
	false, 'A contenteditable ancestor keeps Space');
equal(press(new TestHTMLElement('DIV'), ' ', true), true, 'Handled events stay handled');
equal(toggles, 0, 'Controls never toggle playback');
equal(press(new TestHTMLElement('MAIN')), true, 'Background Space is consumed');
equal(press(null, ';'), true, 'Background semicolon is consumed');
equal(toggles, 2, 'Background shortcuts toggle playback once each');
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const focus = (element: TestHTMLElement) => {
	focused = element;
	listeners.get('focusin:false')!({ target: element });
};
const clicked = new TestHTMLElement('BUTTON');
listeners.get('pointerdown:true')!({});
focus(clicked);
equal(focused, clicked, 'Pointer focus stays put until the next tick so dragging can start');
await tick();
equal(focused, null, 'Pointer-driven focus is dropped on the next tick');
listeners.get('keydown:true')!({ key: 'Tab' });
focus(clicked);
await tick();
equal(focused, clicked, 'Tab-driven focus is retained');
equal(press(clicked), false, 'A keyboard-focused button keeps native Space');
listeners.get('pointerdown:true')!({});
const dialogButton = new TestHTMLElement('BUTTON', new TestHTMLElement('DIV', null, { role: 'dialog' }));
focus(dialogButton);
await tick();
equal(focused, dialogButton, 'Dialog focus is preserved even after a click');
cleanup();
equal(handler, undefined, 'Cleanup removes the listener');
equal(listeners.size, 0, 'Cleanup removes all pointer, capture and focus listeners');
console.log('shortcuts.check: ok');
