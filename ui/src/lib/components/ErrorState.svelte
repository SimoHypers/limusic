<script lang="ts">
	// Every failed page load in the app lands here, so this is where a Rust error stops being a
	// string and starts being something a user can act on. A dead wifi used to render as
	// `http: error sending request for url (https://music.youtube.com/youtubei/v1/browse)` in red.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { RefreshIcon, Alert02Icon, NoInternetIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { isUnreachable } from '$lib/neterr';
	import { t } from '$lib/i18n.svelte';

	let { message, onRetry }: { message: string; onRetry?: () => void } = $props();

	const offline = $derived(isUnreachable(message));
	// Anything we can't name keeps its raw text, one size down and muted: it is the only thing a
	// bug report can carry, and it reads as detail rather than as an alarm.
	const body = $derived(offline ? t('errors.offline_body') : message);
</script>

<div class="flex items-start gap-3 py-6">
	<span class="mt-0.5 shrink-0 rounded-full bg-muted p-2 text-muted-foreground">
		<!-- altIcon/showAlt, not a ternary: `icon` is read once at mount, and a retry can land on a
		     different kind of failure than the one that mounted this. -->
		<HugeiconsIcon
			icon={Alert02Icon}
			altIcon={NoInternetIcon}
			showAlt={offline}
			class="h-5 w-5"
		/>
	</span>
	<div class="flex min-w-0 flex-col items-start gap-1">
		<p class="text-sm font-medium">
			{offline ? t('errors.offline_title') : t('errors.generic_title')}
		</p>
		<p class="max-w-md break-words text-sm text-muted-foreground">{body}</p>
		<!-- shadcn's Button spreads onclick straight onto the real <button>, so passing `onRetry`
		     directly would hand the callback a MouseEvent, and any retry handler with an optional
		     parameter (like home's `load`) would silently receive it as an argument instead of its
		     default. -->
		{#if onRetry}
			<Button variant="outline" size="sm" class="mt-3 gap-2" onclick={() => onRetry()}>
				<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4" />
				{t('common.try_again')}
			</Button>
		{/if}
	</div>
</div>
