/**
 * Reparent a `fixed` popup to <body>.
 *
 * `position: fixed` is only viewport-relative while no ancestor is a containing block for it, and
 * `contain` / `content-visibility` / `transform` / `filter` on any ancestor all make one. Shelf uses
 * content-visibility, so an in-place popup anchored by `anchorMenu` lands at the shelf's offset
 * instead of the viewport's and is clipped away by paint containment: the further down the feed, the
 * further off it goes. At <body> there is nothing above it to contain it.
 */
export function toBody(el: HTMLElement) {
	document.body.appendChild(el);
	return () => el.remove();
}

/** What a popup hangs off: a trigger's box, or a zero-size box at the pointer. */
type Box = { left: number; right: number; top: number; bottom: number };

/** What `fitMenu` needs to place the popup once its real size is known. */
export type Anchor = { style: string; box: Box; gap: number; align: 'left' | 'right' };

/**
 * `transition:none` is load-bearing, not tidiness. The popups carry Tailwind's `duration-150` for
 * their fade, which sets `transition-duration` — and CSS's default `transition-property` is `all`,
 * so the moment `fitMenu` writes `top`, WebKit *transitions* the menu from wherever it was to where
 * it belongs, over 150ms. Measured: a menu with `top:258px` in its inline style rendered at 157 and
 * crept down to 258 over the next 150ms, which is the "slides in from the top" this was reported as.
 */
const NO_TRANSITION = 'transition:none';

/**
 * The style a popup renders with until `fitMenu` measures it. Laid out (so it can be measured) but
 * never painted, because until then the only honest answer to where it goes is "not yet known".
 */
const UNPLACED = `visibility:hidden;${NO_TRANSITION}`;

/** Placeholder for a menu that has never been opened. */
export const NO_ANCHOR: Anchor = {
	style: UNPLACED,
	box: { left: 0, right: 0, top: 0, bottom: 0 },
	gap: 0,
	align: 'left'
};

/** Closest the popup may come to the window edge. */
const EDGE = 8;

/**
 * Inline style placing a `w` x `h` popup against `box`: below it, or above when there isn't room
 * below and there is above; hanging off its left or right edge; and never closer to the window
 * edge than EDGE, whatever the anchor asked for. All four offsets are named so the result can
 * replace `UNPLACED` wholesale, and the transform origin is whichever corner ends up at the anchor,
 * so the open animation grows out of the pointer (or the ⋯) rather than out of thin air.
 */
export function place(a: Anchor, w: number, h: number) {
	const { box, gap, align } = a;
	const { innerWidth: vw, innerHeight: vh } = window;
	const up = box.bottom + gap + h > vh && box.top - gap - h >= 0;
	const right = align === 'right' || box.left + w > vw;
	// Both offsets are distances from an edge, so one clamp does either side.
	const clamp = (v: number, size: number, limit: number) =>
		Math.max(EDGE, Math.min(v, limit - size - EDGE));
	const x = clamp(right ? vw - box.right : box.left, w, vw);
	const y = clamp(up ? vh - box.top + gap : box.bottom + gap, h, vh);
	return (
		`${NO_TRANSITION};left:${right ? 'auto' : x + 'px'};right:${right ? x + 'px' : 'auto'};` +
		`top:${up ? 'auto' : y + 'px'};bottom:${up ? y + 'px' : 'auto'};` +
		`transform-origin:${up ? 'bottom' : 'top'} ${right ? 'right' : 'left'}`
	);
}

/**
 * Record what a `fixed` menu should hang off: a click on a ⋯ trigger anchors to the trigger's box
 * (`e.currentTarget`), a right-click anchors to the pointer. `align: 'right'` lines the menu's
 * right edge up with the trigger's, which is what a ⋯ button wants.
 *
 * Nothing is placed here — the size isn't known yet. `fitMenu` on the popup does the placing.
 */
export function anchorMenu(e: Event, { align = 'left' }: { align?: 'left' | 'right' } = {}): Anchor {
	const atPointer = e.type === 'contextmenu';
	const { clientX: px, clientY: py } = e as MouseEvent;
	const box = atPointer
		? { left: px, right: px, top: py, bottom: py }
		: (e.currentTarget as HTMLElement).getBoundingClientRect();
	return { style: UNPLACED, box, gap: atPointer ? 0 : 4, align };
}

/**
 * Attachment for the popup itself: measure it, then place it, in the same tick it was created and
 * so before anything is painted. Sizes can't be guessed — a ten-item track menu is 380px tall and
 * "Remove from Liked Songs" makes it 260 wide — and a guess that has to be corrected is a menu that
 * moves after you can already see it.
 *
 * Measuring happens at the top-left corner: a `fixed` box shrink-wraps to the room left of the
 * window edge, so a menu measured where it will sit reports the squeezed width, not its own.
 */
export function fitMenu(a: Anchor) {
	return (el: HTMLElement) => {
		el.style.cssText = `${UNPLACED};left:0;top:0;right:auto;bottom:auto`;
		// offset*, not a client rect: the popup opens under a `zoom-in-95` animation, and a client
		// rect is the *transformed* box, so it would report 95% of the real size.
		const { offsetWidth, offsetHeight } = el;
		el.style.cssText = place(a, offsetWidth, offsetHeight);
	};
}

/**
 * Where WebKit's own menu still earns its place: a text field, a selection sitting under the
 * pointer, and a held Shift. The first two carry cut/copy/paste; everywhere else the menu is
 * back / reload / inspect, which is nothing you can do to a song.
 */
function wantsNative(e: MouseEvent) {
	// Shift+right-click is the browser convention for "give me the real menu anyway", and it is how
	// you still reach Inspect Element in a dev build.
	if (e.shiftKey) return true;
	const t = e.target;
	if (!(t instanceof Element)) return false;
	if (t.closest('input, textarea, [contenteditable]')) return true;
	const sel = window.getSelection();
	return !!sel && !sel.isCollapsed && sel.containsNode(t, true);
}

/**
 * Window-level handler that takes the native menu away everywhere it is useless. Our own menus
 * (`ctxHost` below) stop the event before it reaches here, so this only ever sees the leftovers.
 */
export function suppressNative(e: MouseEvent) {
	if (!wantsNative(e)) e.preventDefault();
}

/**
 * Attachment for a ⋯ trigger: right-clicking anywhere inside its nearest `[data-ctx]` ancestor
 * opens the same menu at the pointer. The host marks the region; the menu that lives inside it does
 * the wiring, so a host only ever grows one attribute.
 *
 * A host is *an item* — a track row, a card, a library row, the now-playing block. Page headers,
 * toolbars and hero artwork are not: they carry a ⋯ button of their own in plain sight, and making
 * half a page right-clickable only makes it unclear what the menu is even about.
 *
 * Buttons inside an item are their own thing too, so right-clicking Like does nothing rather than
 * opening the row's menu. The ⋯ trigger and artwork marked `data-ctx-button` are exceptions.
 *
 * Nested hosts are fine: the innermost one wins, because `open` stops the event.
 */
export function ctxHost(open: (e: MouseEvent) => void) {
	return (trigger: HTMLElement) => {
		const host = trigger.closest('[data-ctx]');
		if (!host) return;
		const onContextMenu = (e: Event) => {
			if (wantsNative(e as MouseEvent)) return; // let the field keep its paste menu
			const t = e.target;
			const button = t instanceof Element ? t.closest('button') : null;
			if (button && button !== trigger && !button.hasAttribute('data-ctx-button')) return;
			open(e as MouseEvent);
		};
		host.addEventListener('contextmenu', onContextMenu);
		return () => host.removeEventListener('contextmenu', onContextMenu);
	};
}
