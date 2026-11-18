<script lang="ts">
	import { AlertDialog as AlertDialogPrimitive } from "bits-ui";
	import { cn, type WithoutChild, type WithoutChildrenOrChild } from "$lib/utils.js";
	import AlertDialogOverlay from "./alert-dialog-overlay.svelte";
	import AlertDialogPortal from "./alert-dialog-portal.svelte";
	import type { ComponentProps } from "svelte";

	let {
		ref = $bindable(null),
		class: className,
		size = "default",
		portalProps,
		...restProps
	}: WithoutChild<AlertDialogPrimitive.ContentProps> & {
		size?: "default" | "sm";
		portalProps?: WithoutChildrenOrChild<ComponentProps<typeof AlertDialogPortal>>;
	} = $props();
</script>

<AlertDialogPortal {...portalProps}>
	<AlertDialogOverlay />
	<!-- Centred by the flex wrapper, never translate(-50%, -50%): same soft-text cause as
	     dialog-content.svelte (#75). -->
	<div class="pointer-events-none fixed inset-0 z-50 flex items-center justify-center">
		<AlertDialogPrimitive.Content
			bind:ref
			data-slot="alert-dialog-content"
			data-size={size}
			class={cn(
				"gap-6 rounded-4xl bg-popover p-6 text-popover-foreground ring-1 ring-foreground/5 duration-100 data-[size=default]:max-w-xs data-[size=sm]:max-w-xs data-[size=default]:sm:max-w-md data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 group/alert-dialog-content pointer-events-auto relative grid w-full outline-none",
				className
			)}
			{...restProps}
		/>
	</div>
</AlertDialogPortal>
