<script lang="ts">
	// Undecorated window (tauri.conf `decorations: false`) = the compositor no longer provides
	// resize borders, so the app recreates them: invisible strips along every edge/corner that
	// hand the mousedown to the compositor's interactive resize. Hidden while maximized (a
	// maximized window has no edges to grab, and neither has a fullscreen one, theater mode) and
	// whenever a system frame is drawing its own (win.chrome), where ours would sit on top of the
	// real ones and swallow the grab. pointer-events-auto: an open dialog sets `pointer-events:
	// none` on <body>, and resizing has to keep working under one (#427).
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { win } from '$lib/win.svelte';
	import { ui } from '$lib/player.svelte';

	type Dir =
		| 'North'
		| 'South'
		| 'East'
		| 'West'
		| 'NorthEast'
		| 'NorthWest'
		| 'SouthEast'
		| 'SouthWest';

	const w = getCurrentWindow();

	function start(e: MouseEvent, dir: Dir) {
		if (e.button !== 0) return;
		e.preventDefault();
		w.startResizeDragging(dir).catch(() => {});
	}

	// Edges are 4px, corners 8px (corners win by being later in the DOM at the overlap).
	const handles: { dir: Dir; cls: string }[] = [
		{ dir: 'North', cls: 'top-0 inset-x-2 h-1 cursor-n-resize' },
		{ dir: 'South', cls: 'bottom-0 inset-x-2 h-1 cursor-s-resize' },
		{ dir: 'West', cls: 'left-0 inset-y-2 w-1 cursor-w-resize' },
		{ dir: 'East', cls: 'right-0 inset-y-2 w-1 cursor-e-resize' },
		{ dir: 'NorthWest', cls: 'top-0 left-0 h-2 w-2 cursor-nw-resize' },
		{ dir: 'NorthEast', cls: 'top-0 right-0 h-2 w-2 cursor-ne-resize' },
		{ dir: 'SouthWest', cls: 'bottom-0 left-0 h-2 w-2 cursor-sw-resize' },
		{ dir: 'SouthEast', cls: 'bottom-0 right-0 h-2 w-2 cursor-se-resize' }
	];
</script>

{#if !win.maximized && !ui.theaterOpen && win.chrome === 'off'}
	{#each handles as h (h.dir)}
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="pointer-events-auto fixed z-[60] {h.cls}"
			data-window-control
			onmousedown={(e) => start(e, h.dir)}
			aria-hidden="true"
		></div>
	{/each}
{/if}
