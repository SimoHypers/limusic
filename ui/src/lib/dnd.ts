// Drag payload for BrowseItems: any card is a drag source, the home Shortcuts grid is the drop
// target. One custom MIME type so a drop zone can recognise our drags during `dragover`, where the
// spec forbids reading the data itself — only the type list is visible until the drop lands.
import type { BrowseItem } from './api';

export const ITEM_MIME = 'application/x-limusic-item';
/** A queue row being dragged to a new position (`QueueList`), carrying its queue index. */
export const QUEUE_ROW_MIME = 'application/x-limusic-queue-row';
/** A home section being reordered in the Arrange home modal (`HomeLayoutDialog`). */
export const SECTION_ROW_MIME = 'application/x-limusic-section-row';

export function setDragItem(e: DragEvent, item: BrowseItem): void {
	e.dataTransfer?.setData(ITEM_MIME, JSON.stringify(item));
}

/** True during dragover when this drag is one of ours (a file or a link is not). */
export const isDragItem = (e: DragEvent): boolean => !!e.dataTransfer?.types.includes(ITEM_MIME);

/** The dropped item, or null when the payload isn't ours or is malformed. */
export function getDragItem(e: DragEvent): BrowseItem | null {
	try {
		const raw = e.dataTransfer?.getData(ITEM_MIME);
		if (!raw) return null;
		const item = JSON.parse(raw) as BrowseItem;
		return typeof item?.id === 'string' && typeof item?.title === 'string' ? item : null;
	} catch {
		return null; // not JSON — some other app's drag claiming our type
	}
}

// --- edge auto-scroll -----------------------------------------------------------------------

/** How far from an edge (px) the pull starts, and how hard it pulls right at the edge (px/second). */
const EDGE = 96;
const SPEED = 1800;
/** How often the scroll steps, and the longest step it will take after a stall (ms). */
const TICK = 16;
const MAX_STEP = 100;

/**
 * Scroll speed in px/second for a pointer at `y` inside a box spanning `top`..`bottom`: 0 in the
 * middle, ramping to ±SPEED at each edge and staying there past it (the pointer can leave the box
 * entirely while the button is still down). Negative scrolls up. Pure — the loop below is the only
 * impure part.
 */
export function edgeVelocity(y: number, top: number, bottom: number): number {
	const up = y - top;
	if (up < EDGE) return -SPEED * (1 - Math.max(up, 0) / EDGE);
	const down = bottom - y;
	if (down < EDGE) return SPEED * (1 - Math.max(down, 0) / EDGE);
	return 0;
}

/**
 * Svelte attachment for a scroll container: while a drag carrying `mime` is in flight, aiming at
 * the container's top or bottom edge scrolls it. The drop target (home's Shortcuts grid) lives at
 * the top of the page, so without this you can only ever drag something already on screen with it.
 * Per-mime, so a queue reorder doesn't also scroll the page behind the panel, and vice versa.
 *
 * Listens on the document, not the node — a drag over a *different* element still has to move this
 * container, and `dragover` bubbles all the way up. The dragover events can't drive the scroll
 * themselves: they stop firing when the pointer holds still, which is exactly when someone parked at
 * the edge is waiting for it to happen. So they only record where the cursor is, and a timer does
 * the scrolling.
 *
 * A timer and not rAF, moving by elapsed time and not by a fixed step per tick: WebKitGTK runs the
 * drag inside its own nested event loop, which starves the animation frame callback badly enough
 * that a per-frame step crawls. Distance per *second* is what has to stay constant here.
 */
export function dragScroll(el: HTMLElement, mime: string = ITEM_MIME) {
	let y: number | null = null;
	let timer: ReturnType<typeof setInterval> | undefined;
	let last = 0;
	/** A drag whose terminating event never reaches `document` (a source row unmounted by the
	 *  windowed queue list, or WebKitGTK's nested drag loop swallowing it) used to leave this
	 *  interval running at 60 Hz forever, forcing layout on every tick. `dragover` fires
	 *  continuously while a drag is live, so going quiet for this long means the drag is over.
	 *  Generous, because the comment above says WebKitGTK stops sending them when the pointer holds
	 *  still: this has to outlast a parked-at-the-edge drag, and 3s still bounds a stranded timer. */
	const IDLE_STOP = 3000;
	let lastOver = 0;

	function step() {
		if (y === null) return;
		const now = performance.now();
		if (now - lastOver > IDLE_STOP) return stop();
		// Capped: after a long stall, catching up in one jump throws the page past what you aimed at.
		const dt = Math.min(now - last, MAX_STEP) / 1000;
		last = now;
		const box = el.getBoundingClientRect();
		el.scrollTop += edgeVelocity(y, box.top, box.bottom) * dt;
	}

	function over(e: DragEvent) {
		if (!e.dataTransfer?.types.includes(mime)) return; // a file, a link, or someone else's drag
		y = e.clientY;
		lastOver = performance.now();
		if (timer === undefined) {
			last = performance.now();
			timer = setInterval(step, TICK);
		}
	}

	function stop() {
		y = null;
		clearInterval(timer);
		timer = undefined;
	}

	document.addEventListener('dragover', over);
	document.addEventListener('drop', stop);
	document.addEventListener('dragend', stop);
	return () => {
		stop();
		document.removeEventListener('dragover', over);
		document.removeEventListener('drop', stop);
		document.removeEventListener('dragend', stop);
	};
}

// --- foreign drags --------------------------------------------------------------------------

/**
 * Swallow a drag that isn't ours. The window is built with `dragDropEnabled: false` (Tauri's own
 * handler blocks HTML5 drag and drop on Windows, which is what the queue reorder and the Shortcuts
 * grid are built on), and with nothing catching them a file or a link dropped on the window
 * navigates the webview to it and blanks the app. Ours carry an `x-limusic` type and pass through
 * untouched, so the "no drop" cursor still shows where a card can't land.
 */
export function blockForeignDrag(e: DragEvent): void {
	if (!e.dataTransfer?.types.some((t) => t.startsWith('application/x-limusic'))) e.preventDefault();
}
