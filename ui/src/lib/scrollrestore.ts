// Putting a scroller back where a back/forward navigation left it. Used by the layout for every
// route that scrolls `<main>`, and by /playlist/[id], which scrolls its own container.
//
// `restoreTo` re-applies the saved offset each frame while the destination is filling in. A
// single reach isn't enough: a shelf or an image filling in late shifts the layout under a
// pixel-perfect position. So it releases once the height stays quiet for ~300ms, after ~2.5s
// of re-applying, or when the user steers: wheel, touch, pointer, keys, or a drag.
//
// One restore at a time: starting one cancels the last, which is what a page restoring its own
// scroller after the layout restored `<main>` wants.
let restoreRaf = 0;

export const cancelRestore = () => {
	cancelAnimationFrame(restoreRaf);
	removeEventListener('wheel', cancelRestore, { capture: true });
	removeEventListener('touchstart', cancelRestore, { capture: true });
	removeEventListener('pointerdown', cancelRestore, { capture: true });
	removeEventListener('keydown', cancelRestore, { capture: true });
	removeEventListener('dragstart', cancelRestore, { capture: true });
};

export const restoreTo = (el: HTMLElement, y: number) => {
	cancelRestore();
	// The settle loop writes `y` every frame, so any input that changes the scroll while it
	// runs has to abort it. wheel/touchstart cover a wheel and a touchscreen; a scrollbar
	// drag and the keyboard fire pointerdown/keydown instead; dragScroll on `<main>` runs off
	// dragover while a card is in flight, so dragstart catches it.
	addEventListener('wheel', cancelRestore, { passive: true, capture: true });
	addEventListener('touchstart', cancelRestore, { passive: true, capture: true });
	addEventListener('pointerdown', cancelRestore, { passive: true, capture: true });
	addEventListener('keydown', cancelRestore, { passive: true, capture: true });
	addEventListener('dragstart', cancelRestore, { passive: true, capture: true });
	let lastHeight = el.scrollHeight;
	let quiet = 0;
	let budget = 0;
	const settle = () => {
		el.scrollTop = y;
		const grown = el.scrollHeight > lastHeight;
		lastHeight = el.scrollHeight;
		// Release when the scroller can hold y *and* the content has stopped changing for
		// ~300ms: a late shelf or an image filling in shifts the layout under a pixel
		// position, so a single reach isn't stable. Still clamped short of y, it keeps waiting:
		// the playlist page pulls its next page in only as the bottom comes into view, and one
		// page can take longer than the quiet window. A list that got genuinely shorter sits at
		// the clamped bottom until the budget runs out. Give up after ~2.5s overall (budget
		// counts every frame and never resets, so a destination that keeps growing occupies the
		// position for at most that long), or the user steers: wheel, touch, pointer, keys, or a
		// drag all cancel.
		if (++budget > 150) return cancelRestore();
		if (!grown && el.scrollTop + 1 >= y) {
			if (++quiet > 18) return cancelRestore();
		} else {
			quiet = 0;
		}
		restoreRaf = requestAnimationFrame(settle);
	};
	settle();
};
