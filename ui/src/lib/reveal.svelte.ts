// Chunked reveal for open-ended card grids.
//
// `.card-grid` already sets `content-visibility: auto`, so a card the user has not scrolled to
// costs no layout and no paint. What it still costs is the mount: a `MediaCard` carries a menu
// component with several deriveds of its own, and a large library builds every one of them in one
// synchronous pass on the tab click. So render a chunk, and reveal the next when a sentinel below
// the grid comes into range. Same shape as the playlist page's continuation sentinel, minus the
// network.
//
// Below `MIN_TO_CHUNK` nothing is held back: a grid you can see the end of gains nothing from this
// and a sentinel is one more observer for no reason. Same call the queue panel makes
// (`QueueList.svelte`, WINDOW_ABOVE).

/** Grids at or below this render whole, exactly as they always did. */
const MIN_TO_CHUNK = 200;
/** How many more cards each approach to the bottom reveals. */
const CHUNK = 120;

export function reveal() {
	let shown = $state(CHUNK);

	return {
		/** How many cards to render. Pass the full list's length. */
		count(total: number): number {
			return total <= MIN_TO_CHUNK ? total : Math.min(shown, total);
		},
		/** True when a sentinel is worth rendering under the grid. */
		more(total: number): boolean {
			return total > MIN_TO_CHUNK && shown < total;
		},
		/** Back to one chunk: a different list, or a different tab, starts over. */
		reset() {
			shown = CHUNK;
		},
		/**
		 * Sentinel attachment. The observer only fires on *entering* view, so the chunk it reveals
		 * has to push it back out before the next one can be asked for. rootMargin starts the
		 * reveal early enough that the cards are usually there by the time you reach them.
		 */
		sentinel(node: HTMLElement) {
			const io = new IntersectionObserver(([e]) => e.isIntersecting && (shown += CHUNK), {
				rootMargin: '1800px 0px'
			});
			io.observe(node);
			return () => io.disconnect();
		}
	};
}
