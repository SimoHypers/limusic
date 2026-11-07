<script lang="ts">
	import { onMount } from 'svelte';
	import { appearance } from '$lib/theme.svelte';

	let {
		height,
		calm = false,
		hasHeaderText = false,
		textWidth = 0
	}: {
		height: string;
		calm?: boolean;
		hasHeaderText?: boolean;
		textWidth?: number;
	} = $props();

	let now = $state(new Date());
	let ready = $state(false);

	onMount(() => {
		let timer: ReturnType<typeof setTimeout>;
		const tick = () => {
			now = new Date();
			timer = setTimeout(tick, 60_000 - (now.getSeconds() * 1000 + now.getMilliseconds()));
		};
		tick();
		return () => clearTimeout(timer);
	});

	$effect(() => {
		if (!hasHeaderText || textWidth > 0) {
			const id = requestAnimationFrame(() => {
				ready = true;
			});
			return () => cancelAnimationFrame(id);
		}
	});

	const hour12 = $derived(
		appearance.theaterClockFormat === 'auto' ? undefined : appearance.theaterClockFormat === '12'
	);
	const text = $derived(now.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit', hour12 }));

	const positionClass = $derived.by(() => {
		if (appearance.theaterClockSide === 'right') {
			return !calm ? 'right-24 xl:right-28' : 'right-8 xl:right-14';
		}
		return '';
	});

	const sideStyle = $derived.by(() => {
		if (appearance.theaterClockSide === 'left') {
			const offset = !calm && hasHeaderText && textWidth > 0 ? textWidth + 24 : 0;
			return `left: calc(var(--pad) + ${offset}px);`;
		}
		return '';
	});
</script>

<div
	class="pointer-events-none absolute top-0 z-20 flex items-center text-2xl font-medium tabular-nums text-foreground/60 [--pad:2rem] xl:[--pad:3.5rem] {positionClass} {ready
		? 'transition-[left,right] duration-500 ease-in-out'
		: 'transition-none'}"
	style="height:{height}; {sideStyle} {appearance.theaterClockFont
		? `font-family:${appearance.theaterClockFont}`
		: ''}"
>
	{text}
</div>