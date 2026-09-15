/** Ephemeral viewport state owned by a queue view, never persisted as a user preference. */
export interface QueueScrollMemory {
	position?: {
		scrollTop: number;
		queue: object;
		currentIndex: number;
		historyVisible: boolean;
	};
}
/** Keep the queue's current-track heading in place when history changes visibility.
 *  Measure the remaining displacement after rendering: Chromium may already have anchored
 *  the scroll position, while WebKit needs the entire correction. */
type Anchor = { isConnected: boolean; getBoundingClientRect(): { top: number } };
type Scroller = Anchor & { scrollTop: number };

export function keepQueueAnchor(
	scroller: Scroller,
	heading: Anchor,
	rendered: Promise<void>,
	isCurrent: () => boolean
): () => void {
	const before = heading.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
	let cancelled = false;
	rendered.then(() => {
		if (cancelled || !scroller.isConnected || !heading.isConnected || !isCurrent()) return;
		const after = heading.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
		scroller.scrollTop += after - before;
	});
	// An effect rerun or unmount must not adjust a newer view after its pending tick resolves.
	return () => {
		cancelled = true;
	};
}
