<script lang="ts">
	import { Dialog as DialogPrimitive } from "bits-ui";
	import DialogPortal from "./dialog-portal.svelte";
	import type { Snippet } from "svelte";
	import * as Dialog from "./index.js";
	import { cn, type WithoutChildrenOrChild } from "$lib/utils.js";
	import type { ComponentProps } from "svelte";
	import { Button } from "$lib/components/ui/button/index.js";
	import { HugeiconsIcon } from "@hugeicons/svelte"
	import { Cancel01Icon } from '@hugeicons/core-free-icons';
	import { t } from '$lib/i18n.svelte';

	let {
		ref = $bindable(null),
		class: className,
		portalProps,
		children,
		showCloseButton = true,
		...restProps
	}: WithoutChildrenOrChild<DialogPrimitive.ContentProps> & {
		portalProps?: WithoutChildrenOrChild<ComponentProps<typeof DialogPortal>>;
		children: Snippet;
		showCloseButton?: boolean;
	} = $props();
</script>

<DialogPortal {...portalProps}>
	<Dialog.Overlay />
	<!-- Centred by the flex wrapper, never with translate(-50%, -50%): a half-pixel transform
	     offset (any dialog with an odd height) is not pixel-snapped, so WebKitGTK's compositor
	     resamples the whole layer and the text goes soft. See issue #75. Not inset-0 + m-auto +
	     h-fit either: WebKitGTK ignores height:fit-content there and stretches to the viewport. -->
	<div class="pointer-events-none fixed inset-0 z-50 flex items-center justify-center">
		<DialogPrimitive.Content
			bind:ref
			data-slot="dialog-content"
			class={cn(
				"bg-popover text-popover-foreground data-open:animate-in data-closed:animate-out data-closed:fade-out-0 data-open:fade-in-0 data-closed:zoom-out-95 data-open:zoom-in-95 ring-foreground/5 grid max-w-[calc(100%-2rem)] gap-6 rounded-4xl p-6 text-sm ring-1 duration-100 sm:max-w-md pointer-events-auto relative w-full outline-none",
				className
			)}
			{...restProps}
		>
			{@render children?.()}
			{#if showCloseButton}
				<DialogPrimitive.Close data-slot="dialog-close">
					{#snippet child({ props })}
						<Button variant="ghost" class="absolute top-4 right-4" size="icon-sm" {...props}>
							<HugeiconsIcon icon={Cancel01Icon} strokeWidth={2}  />
							<span class="sr-only">{t('common.close')}</span>
						</Button>
					{/snippet}
				</DialogPrimitive.Close>
			{/if}
		</DialogPrimitive.Content>
	</div>
</DialogPortal>
