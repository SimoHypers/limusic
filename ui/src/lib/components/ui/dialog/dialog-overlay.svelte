<script lang="ts">
	import { Dialog as DialogPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		...restProps
	}: DialogPrimitive.OverlayProps = $props();
</script>

<!-- No backdrop-filter: at 80% black the blur was barely visible, and a full-window one is re-run
     every time anything under it repaints, four times a second while a song plays (#341).
     z-[49], one under the titlebar's z-50, so the window stays draggable and closable (#427). -->
<DialogPrimitive.Overlay
	bind:ref
	data-slot="dialog-overlay"
	class={cn("data-open:animate-in data-closed:animate-out data-closed:fade-out-0 data-open:fade-in-0 bg-black/80 duration-100 fixed inset-0 isolate z-[49]", className)}
	{...restProps}
/>
