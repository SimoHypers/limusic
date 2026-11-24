<script lang="ts">
	import {
		SIDE_PANEL_MAX,
		SIDE_PANEL_MIN,
		dragPanelWidth
	} from '$lib/sidepanel';
	import {
		resetSidePanelWidth,
		setSidePanelWidth,
		sidePanel
	} from '$lib/sidepanel.svelte';
	import { t } from '$lib/i18n.svelte';

	let drag: { startX: number; startWidth: number } | null = null;
	let dragging = $state(false);

	function onPointerDown(e: PointerEvent) {
		if (e.button !== 0) return;
		drag = { startX: e.clientX, startWidth: sidePanel.width };
		dragging = true;
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		e.preventDefault();
	}

	function onPointerMove(e: PointerEvent) {
		if (!drag) return;
		setSidePanelWidth(dragPanelWidth(drag.startWidth, drag.startX, e.clientX));
	}

	function onPointerUp() {
		drag = null;
		dragging = false;
	}

	function onKeyDown(e: KeyboardEvent) {
		const step = e.shiftKey ? 40 : 16;
		if (e.key === 'ArrowLeft') setSidePanelWidth(sidePanel.width + step);
		else if (e.key === 'ArrowRight') setSidePanelWidth(sidePanel.width - step);
		else if (e.key === 'Home') resetSidePanelWidth();
		else return;
		e.preventDefault();
		e.stopPropagation();
	}
</script>

<!-- Left-edge resize handle for right-docked side panels. Double-clicking resets to default. -->
<div
	role="slider"
	tabindex="0"
	aria-orientation="vertical"
	aria-label={t('a11y.resize_side_panel')}
	aria-valuemin={SIDE_PANEL_MIN}
	aria-valuemax={SIDE_PANEL_MAX}
	aria-valuenow={sidePanel.width}
	class="group absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize touch-none outline-none"
	onpointerdown={onPointerDown}
	onpointermove={onPointerMove}
	onpointerup={onPointerUp}
	onpointercancel={onPointerUp}
	ondblclick={resetSidePanelWidth}
	onkeydown={onKeyDown}
>
	<span
		class="pointer-events-none absolute inset-y-0 left-1 w-0.5 transition-colors group-hover:bg-primary/60 group-focus-visible:bg-primary {dragging
			? 'bg-primary'
			: ''}"
	></span>
</div>
