<script lang="ts">
	// Saturation/value square + hue slider + hex field. Ported down from Kibo UI's React component:
	// no alpha (a translucent --primary just breaks the theme), no format dropdown (HEX/RGB/HSL is a
	// dev-facing control), and the hex field is editable here — theirs is read-only, which blocks the
	// one thing people actually want to do, paste a hex.
	//
	// The square is a pointer target with no keyboard equivalent; the hex field is the accessible
	// path to any colour, and the hue slider is a real slider (arrow keys work).
	import { untrack } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ColorPickerIcon } from '@hugeicons/core-free-icons';
	import { Slider } from '$lib/components/ui/slider';
	import { Input } from '$lib/components/ui/input';
	import { hexToHsv, hsvToHex, type Hsv } from '$lib/color';
	import { t } from '$lib/i18n.svelte';

	let { value, onchange }: { value: string; onchange: (hex: string) => void } = $props();

	// HSV is the picker's own state, not derived from `value` on every frame: hex can't say where the
	// cursor sat (every hue is black at v=0), so a round-trip per drag would make the square jump.
	let hsv = $state<Hsv>(untrack(() => hexToHsv(value)) ?? { h: 0, s: 1, v: 1 });
	let text = $state(untrack(() => value));

	// Reseat only on changes that came from outside (preset switch, reset). Our own edits already
	// satisfy this equality, so they don't feed back.
	$effect(() => {
		const v = value.toLowerCase();
		if (v === untrack(() => hsvToHex(hsv))) return;
		const next = hexToHsv(v);
		if (next) {
			hsv = next;
			text = v;
		}
	});

	const hasEyeDropper = typeof window !== 'undefined' && 'EyeDropper' in window;

	function set(next: Hsv) {
		hsv = next;
		text = hsvToHex(next);
		onchange(text);
	}

	function pick(e: PointerEvent) {
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		const clamp = (n: number) => Math.min(1, Math.max(0, n));
		set({
			...hsv,
			s: clamp((e.clientX - r.left) / r.width),
			v: 1 - clamp((e.clientY - r.top) / r.height)
		});
	}

	function typed(hex: string) {
		text = hex;
		const next = hexToHsv(hex);
		if (next) {
			hsv = next;
			onchange(hsvToHex(next));
		}
	}

	async function eyeDrop() {
		try {
			// @ts-expect-error — EyeDropper is Chromium-only and untyped; feature-detected above.
			const { sRGBHex } = await new window.EyeDropper().open();
			const next = hexToHsv(sRGBHex);
			if (next) set(next);
		} catch {
			// cancelled with Escape — nothing to do
		}
	}
</script>

<div class="flex w-56 flex-col gap-3">
	<!-- No ARIA role fits a 2D colour field, and "application" is the honest one: keys go to the
	     page, and the hex input below is the accessible route to any colour. -->
	<div
		role="application"
		aria-label={t('a11y.saturation_brightness')}
		class="relative h-32 w-full cursor-crosshair rounded-lg"
		style="background:
			linear-gradient(to top, #000, transparent),
			linear-gradient(to right, #fff, transparent),
			hsl({hsv.h} 100% 50%)"
		onpointerdown={(e) => {
			e.preventDefault();
			e.currentTarget.setPointerCapture(e.pointerId);
			pick(e);
		}}
		onpointermove={(e) => {
			if (e.currentTarget.hasPointerCapture(e.pointerId)) pick(e);
		}}
	>
		<div
			class="pointer-events-none absolute size-4 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white shadow-[0_0_0_1px_rgba(0,0,0,0.5)]"
			style="left:{hsv.s * 100}%; top:{(1 - hsv.v) * 100}%"
		></div>
	</div>

	<Slider
		type="single"
		aria-label={t('a11y.hue')}
		max={360}
		step={1}
		value={hsv.h}
		onValueChange={(h) => set({ ...hsv, h })}
		class="[&_[data-slot=slider-range]]:bg-transparent [&_[data-slot=slider-track]]:bg-[linear-gradient(to_right,#f00,#ff0,#0f0,#0ff,#00f,#f0f,#f00)]"
	/>

	<div class="flex items-center gap-2">
		{#if hasEyeDropper}
			<button
				type="button"
				onclick={eyeDrop}
				aria-label={t('a11y.pick_colour')}
				class="flex size-8 shrink-0 items-center justify-center rounded-md border text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
			>
				<HugeiconsIcon icon={ColorPickerIcon} size={16} />
			</button>
		{/if}
		<Input
			value={text}
			oninput={(e) => typed(e.currentTarget.value)}
			aria-label={t('a11y.hex_colour')}
			spellcheck={false}
			class="h-8 bg-secondary px-2 font-mono text-xs shadow-none"
		/>
	</div>
</div>
