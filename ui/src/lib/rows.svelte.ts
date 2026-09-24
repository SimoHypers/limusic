// The DOM half of the row windowing in `rows.ts`: what a scrolling container has to report for the
// maths to run, and how it finds out.
import { ROW_PX } from './rows';

/**
 * Watch a scrolling container: its scroll position, its height, and how tall one row actually is.
 *
 * Row height is measured rather than assumed, which `ROW_PX` alone got wrong. The same `TrackRow`
 * is 56px on a playlist page and 72px in the 320px-wide queue panel. It used to vary within one
 * list as well, because `content-visibility` makes a row report its `contain-intrinsic-size`
 * (3.5rem) while it is skipped and its real height once it has been rendered; reserving 56px for
 * rows that draw at 72px made the scroll height move as you scrolled, which is exactly the thing
 * that makes a windowed list feel broken. Neither list that windows asks for `content-visibility`
 * any more (`TrackRow`'s `lazy`), so that half is gone, but the per-list difference remains.
 *
 * Rows opt in with `data-row`, so this measures a row and not whatever markup happens to be first.
 *
 * Usage:
 *
 *     const sc = rowScroller();
 *     const win = $derived(rowWindow(sc.scrollTop, sc.viewportPx, items.length, sc.rowPx));
 *     <div class="overflow-y-auto" {@attach sc.attach}> … <div data-row> … </div> … </div>
 */
export function rowScroller() {
	let scrollTop = $state(0);
	let viewportPx = $state(0);
	let rowPx = $state(ROW_PX);
	let offsetPx = $state(0);

	return {
		get scrollTop() {
			return scrollTop;
		},
		get viewportPx() {
			return viewportPx;
		},
		get rowPx() {
			return rowPx;
		},
		/**
		 * Distance from the container's scroll origin down to the rows, px: 0 unless the container
		 * holds something above them (a page header that scrolls away) marked `data-rows`.
		 */
		get offsetPx() {
			return offsetPx;
		},
		attach: (node: HTMLElement) => {
			// Chromium anchors the scroll position to a node in view and corrects scrollTop when the
			// content above it changes height. A windowed list changes exactly that on every scroll
			// (rows above the viewport become padding), and any pixel of drift between the padding
			// and the rows it stands in for makes the correction non-zero: the correction fires a
			// scroll event, which moves the window, which shifts the content again, and the list
			// stutters on by itself after the wheel has stopped (issue #87, Windows/WebView2 only,
			// since WebKitGTK anchors nothing). QueueList.togglePrev compensates by hand for the
			// same reason and would otherwise be corrected twice here.
			node.style.overflowAnchor = 'none';
			const read = () => {
				scrollTop = node.scrollTop;
				viewportPx = node.clientHeight;
				// One layout read per scroll frame, on a box the browser has just laid out anyway.
				//
				// Largest seen, not latest. A row only ever under-reports: `contain-intrinsic-size`
				// on TrackRow lands on the content box, so a row WebKit has skipped answers 56px
				// while the same row occupies 72px once it is drawn. Taking the max settles on the
				// drawn height in one step and cannot then oscillate between the two.
				const h = node.querySelector('[data-row]')?.getBoundingClientRect().height ?? 0;
				if (h > rowPx) rowPx = h;
				// Where row 0 sits, for a container that scrolls a header away above the rows. Latest,
				// not largest: the header genuinely changes height (expanding a description), and a
				// stale offset would put the window in the wrong place.
				const rows = node.querySelector('[data-rows]');
				offsetPx = rows
					? rows.getBoundingClientRect().top - node.getBoundingClientRect().top + node.scrollTop
					: 0;
			};
			read();
			// Again after a frame: the first read can land before any row has been rendered, where
			// a skipped row still answers with its intrinsic size rather than its real one.
			const frame = requestAnimationFrame(read);
			node.addEventListener('scroll', read, { passive: true });
			// The container's own box changes on a window resize, never on a scroll, so this is cheap.
			const ro = new ResizeObserver(read);
			ro.observe(node);
			return () => {
				cancelAnimationFrame(frame);
				node.removeEventListener('scroll', read);
				ro.disconnect();
			};
		}
	};
}
