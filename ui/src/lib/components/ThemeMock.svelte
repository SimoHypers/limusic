<script lang="ts">
	// A miniature of the app (sidebar, a shelf of cards, the player bar) in one palette. It carries
	// `theme-<id>` and `dark` itself, so every token re-resolves for this subtree alone, and the
	// overrides land as inline vars here the way `apply` puts them on <html>. That is what lets the
	// Appearance tab follow a slider live: dragging restyles a few dozen elements instead of the
	// whole document (docs/UI-PERFORMANCE.md, "Per-track and per-tick writes land on a leaf").
	//
	// Sized in em off the root's font-size. Radii are scaled by --k: a real corner on a card this
	// small would read as a pill.
	import { m3Seed, onAccent, type Custom, type ThemeId } from '$lib/theme.svelte';

	type Overrides = Pick<Custom, 'accent' | 'hue' | 'radius' | 'fontSans' | 'fontHeading'>;

	let {
		id,
		dark,
		overrides,
		detail
	}: {
		id: ThemeId;
		dark: boolean;
		overrides: Overrides;
		detail: { heading: string; title: string; artist: string; play: string; cover?: string };
	} = $props();

	const isM3 = $derived(id === 'material');
	const fg = $derived(!isM3 && overrides.accent ? onAccent(overrides.accent) : null);
	const seed = $derived(isM3 ? m3Seed(overrides.accent) : null);
	let coverFailed = $state(false);
	$effect(() => {
		void detail.cover;
		coverFailed = false;
	});
</script>

<div
	class="mock theme-{id} {dark ? 'dark' : ''}"
	style:--primary={isM3 ? null : overrides.accent}
	style:--primary-foreground={fg}
	style:--accent={isM3 ? null : overrides.accent}
	style:--accent-foreground={fg}
	style:--m3-seed-h={seed ? seed.h : null}
	style:--m3-c={seed ? seed.c : null}
	style:--hue={overrides.hue !== null ? String(overrides.hue) : seed ? seed.h : null}
	style:--radius={overrides.radius === null ? null : `${overrides.radius}rem`}
	style:--font-sans={overrides.fontSans}
	style:--font-heading={overrides.fontHeading}
	aria-hidden="true"
>
	<div class="body">
		<div class="side">
			<i class="logo"></i>
			<i class="nav on"></i>
			<i class="nav"></i>
			<i class="nav short"></i>
			<i class="nav"></i>
		</div>
		<div class="main">
			<p class="heading">{detail.heading}</p>
			<span class="pill">{detail.play}</span>
			<!-- Cut off at the bottom when it doesn't fit, like a page that scrolls on. -->
			<div class="shelf">
				{#each [0, 1, 2] as n (n)}
					<div class="card">
						<i class="art"></i>
						<i class="line"></i>
						<i class="line faint"></i>
					</div>
				{/each}
			</div>
		</div>
	</div>
	<div class="bar">
		{#if detail.cover && !coverFailed}
			<img class="cover" src={detail.cover} alt="" onerror={() => (coverFailed = true)} />
		{:else}
			<i class="cover"></i>
		{/if}
		<div class="meta">
			<span class="title">{detail.title}</span>
			<span class="artist">{detail.artist}</span>
		</div>
		<div class="track"><i class="fill"></i></div>
		<i class="play"></i>
	</div>
</div>

<style>
	/* Every colour is a token, so the miniature is whatever palette its classes resolve to. */
	.mock {
		--k: 0.7;
		display: flex;
		flex-direction: column;
		aspect-ratio: 16 / 10;
		overflow: hidden;
		font-size: 10px;
		font-family: var(--font-sans);
		color: var(--foreground);
		background: var(--background);
		border: 1px solid var(--border);
		border-radius: calc(var(--radius) * 1.6);
	}
	i {
		display: block;
	}
	.body {
		display: flex;
		flex: 1;
		min-height: 0;
	}
	.side {
		display: flex;
		width: 24%;
		flex-direction: column;
		gap: 0.7em;
		padding: 1em 0.9em;
		background: var(--sidebar);
		border-right: 1px solid var(--sidebar-border);
	}
	.logo {
		width: 1.4em;
		height: 1.4em;
		margin-bottom: 0.4em;
		border-radius: 999px;
		background: var(--primary);
	}
	.nav {
		height: 0.7em;
		border-radius: calc(var(--radius) * var(--k));
		background: var(--muted-foreground);
		opacity: 0.35;
	}
	.nav.on {
		height: 1.4em;
		opacity: 1;
		background: var(--sidebar-accent);
	}
	.nav.short {
		width: 65%;
	}
	.main {
		display: flex;
		min-width: 0;
		flex: 1;
		flex-direction: column;
		gap: 0.9em;
		overflow: hidden;
		padding: 1.1em 1.2em;
	}
	.main > * {
		flex-shrink: 0;
	}
	.heading {
		overflow: hidden;
		font-family: var(--font-heading);
		font-size: 1.45em;
		font-weight: 600;
		line-height: 1.1;
		white-space: nowrap;
		text-overflow: ellipsis;
	}
	.shelf {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: 0.8em;
	}
	.card {
		display: flex;
		flex-direction: column;
		gap: 0.5em;
		padding: 0.6em;
		border: 1px solid var(--border);
		border-radius: calc(var(--radius) * var(--k) * 1.4);
		background: var(--card);
	}
	.art {
		aspect-ratio: 1;
		border-radius: calc(var(--radius) * var(--k));
		background: var(--muted);
	}
	.line {
		height: 0.55em;
		border-radius: 999px;
		background: var(--muted-foreground);
		opacity: 0.45;
	}
	.line.faint {
		width: 60%;
		opacity: 0.25;
	}
	.pill {
		align-self: flex-start;
		padding: 0.35em 1em;
		border-radius: calc(var(--radius) * 2.6 * var(--k));
		background: var(--primary);
		color: var(--primary-foreground);
		font-size: 0.95em;
		font-weight: 600;
	}
	.bar {
		display: flex;
		align-items: center;
		gap: 1em;
		padding: 0.8em 1.2em;
		border-top: 1px solid var(--border);
		background: var(--card);
	}
	.cover {
		width: 2.6em;
		height: 2.6em;
		flex-shrink: 0;
		object-fit: cover;
		border-radius: calc(var(--radius) * var(--k));
		background: var(--muted);
	}
	.meta {
		display: flex;
		width: 26%;
		min-width: 0;
		flex-direction: column;
		gap: 0.35em;
		line-height: 1.15;
	}
	.title,
	.artist {
		overflow: hidden;
		white-space: nowrap;
		text-overflow: ellipsis;
	}
	.title {
		font-weight: 600;
	}
	.artist {
		font-size: 0.85em;
		color: var(--muted-foreground);
	}
	.track {
		flex: 1;
		height: 0.45em;
		overflow: hidden;
		border-radius: 999px;
		background: var(--muted);
	}
	.fill {
		width: 38%;
		height: 100%;
		background: var(--primary);
	}
	.play {
		width: 2.2em;
		height: 2.2em;
		flex-shrink: 0;
		border-radius: 999px;
		background: var(--primary);
	}
	.mock.theme-material .body {
		background: var(--sidebar);
	}
	.mock.theme-material .side {
		border-right-color: transparent;
	}
	.mock.theme-material .main {
		background: var(--background);
		border-top-left-radius: calc(var(--radius) * 1.25);
	}
	.mock.theme-material .nav {
		border-radius: 999px;
	}
	.mock.theme-material .nav.on {
		background: var(--secondary);
	}
	.mock.theme-material .card {
		border-color: transparent;
		background: var(--m3-surface-container, var(--card));
	}
	.mock.theme-material .bar {
		border-top-color: transparent;
		border-radius: calc(var(--radius) * 1.1) calc(var(--radius) * 1.1) 0 0;
		background: var(--m3-surface-container, var(--card));
	}
	.mock.theme-material .track {
		position: relative;
		height: 0.7em;
		overflow: visible;
		background: transparent;
	}
	.mock.theme-material .track::after {
		content: '';
		position: absolute;
		left: calc(38% + 0.35em);
		right: 0;
		top: 50%;
		height: 0.32em;
		transform: translateY(-50%);
		border-radius: 999px;
		background: var(--secondary);
	}
	.mock.theme-material .fill {
		position: relative;
		width: calc(38% - 0.25em);
		height: 100%;
		background: var(--primary);
		-webkit-mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 44 8' preserveAspectRatio='none'%3E%3Cpath d='M 2 4 C 4 4, 5 1.5, 7.5 1.5 S 11 6.5, 14 6.5 S 17.5 1.5, 20.5 1.5 S 24 6.5, 27 6.5 S 30.5 1.5, 33.5 1.5 C 36 1.5, 38 4, 42 4' fill='none' stroke='%23000' stroke-width='2.4' stroke-linecap='round'/%3E%3C/svg%3E");
		-webkit-mask-size: 100% 100%;
		-webkit-mask-repeat: no-repeat;
		mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 44 8' preserveAspectRatio='none'%3E%3Cpath d='M 2 4 C 4 4, 5 1.5, 7.5 1.5 S 11 6.5, 14 6.5 S 17.5 1.5, 20.5 1.5 S 24 6.5, 27 6.5 S 30.5 1.5, 33.5 1.5 C 36 1.5, 38 4, 42 4' fill='none' stroke='%23000' stroke-width='2.4' stroke-linecap='round'/%3E%3C/svg%3E");
		mask-size: 100% 100%;
		mask-repeat: no-repeat;
	}
	.mock.theme-material .track::before {
		content: '';
		position: absolute;
		left: 38%;
		top: 50%;
		width: 0.24em;
		height: 1.15em;
		transform: translate(-50%, -50%);
		border-radius: 999px;
		background: var(--primary);
		z-index: 1;
	}
	.mock.theme-material .play {
		border-radius: calc(var(--radius) * var(--k) * 1.1);
	}
</style>
