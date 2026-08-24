<script lang="ts">
	import { t } from '$lib/i18n.svelte';
	// "E" in a rounded square: YouTube's explicit marker. Drawn here because HugeIcons' free set
	// has no explicit-content icon, to the same 24px box / 1.5 stroke / round joins as the rest of
	// the icons it sits beside, so it takes size and colour from `class` the same way they do.
	let { class: cls = '' }: { class?: string } = $props();
</script>

<svg
	viewBox="0 0 24 24"
	fill="none"
	stroke="currentColor"
	stroke-width="1.5"
	stroke-linecap="round"
	stroke-linejoin="round"
	class={cls}
	role="img"
	aria-label={t('a11y.explicit')}
>
	<rect x="3" y="3" width="18" height="18" rx="4" />
	<path d="M15 7.5H9v9h6M9 12h4.5" />
</svg>
