<script lang="ts">
	// Custom titlebar. The window runs undecorated by default (tauri.conf `decorations: false`);
	// with a system frame (`win.chrome`, issue #65) this bar stays as a toolbar and only drops its
	// own window buttons, since everything else on it is app navigation, not window management.
	// Everything on the bar is a drag region except the buttons; double-click maximizes (Tauri's
	// drag region itself). Right cluster: Last.fm scrobbler | separator | minimize / maximize /
	// close — per the design, the scrobbler lives with the window controls but visually apart.
	// Account (sign in/out) sits first in that cluster, in its own component.
	import { afterNavigate } from '$app/navigation';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ArrowLeft01Icon,
		ArrowRight01Icon,
		Refresh03Icon,
		MinusSignIcon,
		SquareIcon,
		CopyIcon,
		Cancel01Icon,
		Menu01Icon
	} from '@hugeicons/core-free-icons';
	import AccountMenu from './AccountMenu.svelte';
	import IntegrationButtons from './IntegrationButtons.svelte';
	import { appIcon } from '$lib/appicon.svelte';
	import { refreshView, ui } from '$lib/player.svelte';
	import { win } from '$lib/win.svelte';
	import { t } from '$lib/i18n.svelte';

	// `w` is this window; `win` (imported) is the shared frame state.
	const w = getCurrentWindow();


	// Back/forward. `depth` is how many history entries deep the session is, `deepest` how far it
	// has ever been, so both buttons grey out instead of doing nothing. popstate carries a signed
	// delta (the mouse's side buttons come through here); anything else is a push, which wipes the
	// entries ahead of us.
	let depth = $state(0);
	let deepest = $state(0);
	afterNavigate((nav) => {
		if (nav.type === 'enter') depth = deepest = 0;
		else if (nav.delta !== undefined) depth = Math.max(0, depth + nav.delta);
		else deepest = depth += 1;
	});

	// The integrations (link, Listen Together, Discord, Last.fm, theater, mini player) and the
	// Last.fm menu live in IntegrationButtons.svelte: below sm this bar has no room for them and the
	// sidebar drawer renders the same component instead.
</script>

<!-- `relative` makes this a stacking context, so the account/window dropdowns inside it are capped
     at this z: it must outrank the panels below (LyricsPanel/QueuePanel, z-30). Theater mode is
     the exception: it sits at z-40 so dialogs (z-50) can open over it, so the bar has to duck
     under it instead of the other way round. Modal backdrops sit at z-[49], under this bar, so the
     window can still be dragged and closed with a dialog open (#427). pointer-events-auto because
     an open dialog sets `pointer-events: none` on <body>; `data-window-control` marks the buttons
     that don't dismiss it (`onWindowChrome`). -->
<header
	data-tauri-drag-region
	class="pointer-events-auto relative {ui.theaterOpen ? 'z-0' : 'z-50'} flex h-9 shrink-0 select-none items-center justify-between border-b border-border/60 bg-background"
>
	<span
		class="pointer-events-none absolute inset-x-0 text-center text-xs font-medium tracking-wide text-muted-foreground max-sm:hidden"
	>
		Limusic
	</span>

	<!-- macOS overlay style floats the traffic lights over the top-left of the webview, so the row
	     starts clear of them. `trafficLightPosition` in tauri.macos.conf.json centres them in this
	     36px bar and insets them 14px (#189); they end near 68px, so 70px plus the logo's margin
	     leaves about as much space after them as before them. Its `y` is not a top margin: tao
	     sizes the titlebar to the 16px button plus `y` and the buttons keep their offset from its
	     bottom, so their centre lands at y + 2 (y: 10 moved them up 2px on a real Mac). -->
	<div class="flex h-full items-center {win.chrome === 'overlay' ? 'pl-[70px]' : ''}">
		<!-- Below sm the sidebar is off-canvas (see Sidebar.svelte), so this is the only way back to
		     it. It shares the bar with the window controls and nothing else: at 400px the six
		     integration buttons below and this row's refresh button cannot both stay. -->
		<button
			class="flex h-full w-9 items-center justify-center text-foreground/80 transition-colors hover:bg-accent/10 hover:text-foreground sm:hidden"
			onclick={() => (ui.navOpen = !ui.navOpen)}
			aria-expanded={ui.navOpen}
			title={ui.navOpen ? t('a11y.collapse_sidebar') : t('a11y.expand_sidebar')}
			aria-label={ui.navOpen ? t('a11y.collapse_sidebar') : t('a11y.expand_sidebar')}
		>
			<HugeiconsIcon icon={Menu01Icon} strokeWidth={2.5} class="h-5 w-5" />
		</button>
		<!-- pointer-events-none: the logo is decoration; clicks on it should drag the window. -->
		<img src={appIcon.src} alt="" class="pointer-events-none ml-3 mr-1 h-4 w-4" />
		<!-- Bigger and heavier than the icons on the right: these are navigation, and at their
		     weight the arrow read as decoration and got missed. -->
		<button
			class="flex h-full w-9 items-center justify-center text-foreground/80 transition-colors hover:bg-accent/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-25"
			onclick={() => history.back()}
			disabled={depth === 0}
			title={t('common.back')}
			aria-label={t('common.back')}
		>
			<HugeiconsIcon icon={ArrowLeft01Icon} strokeWidth={2.5} class="h-5 w-5" />
		</button>
		<button
			class="flex h-full w-9 items-center justify-center text-foreground/80 transition-colors hover:bg-accent/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-25"
			onclick={() => history.forward()}
			disabled={depth === deepest}
			title={t('common.forward')}
			aria-label={t('common.forward')}
		>
			<HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2.5} class="h-5 w-5" />
		</button>
		<!-- Third in the browser's own control group, and read as such without a label. It drops the
		     browse cache and remounts the page, which is the only way to ask YouTube for a different
		     home feed: refetching rotates roughly a third of "Quick picks" (#177). Never disabled,
		     since the page it reloads is always there; the route's own skeletons are the progress.
		     One of the four this bar keeps at every width, next to the menu, back and forward. -->
		<button
			class="flex h-full w-9 items-center justify-center text-foreground/80 transition-colors hover:bg-accent/10 hover:text-foreground"
			onclick={refreshView}
			title={t('common.refresh')}
			aria-label={t('common.refresh')}
		>
			<HugeiconsIcon icon={Refresh03Icon} strokeWidth={2.5} class="h-4 w-4" />
		</button>
	</div>

	<div class="flex h-full items-center">
		<!-- Account first, then the integrations, then the window controls. The drag region lives on
		     <header> only, so these children are ordinary buttons — don't add the attribute here.
		     Below sm the account moves into the sidebar drawer, next to the theme toggle. -->
		<div class="h-full max-sm:hidden"><AccountMenu /></div>
		<div class="mx-1.5 h-4 w-px bg-border max-sm:hidden"></div>

		<IntegrationButtons />

		{#if win.chrome === 'off'}
			<div class="mx-1.5 h-4 w-px bg-border"></div>

			<!-- Minimize and maximize follow the desktop's button layout (#481). -->
			{#if win.minimize}
				<button
					class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground"
					data-window-control
					onclick={() => w.minimize()}
					aria-label={t('common.minimize')}
				>
					<HugeiconsIcon icon={MinusSignIcon} class="h-4 w-4" />
				</button>
			{/if}
			{#if win.maximize}
				<button
					class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground"
					data-window-control
					onclick={() => w.toggleMaximize()}
					aria-label={win.maximized ? t('common.restore') : t('common.maximize')}
				>
					<!-- Maximized: the restore glyph, two overlapping squares with the front one low-left.
					     That is CopyIcon turned half a turn. altIcon/showAlt, since `icon` is frozen at
					     mount (#428). -->
					<HugeiconsIcon
						icon={SquareIcon}
						altIcon={CopyIcon}
						showAlt={win.maximized}
						class="h-3.5 w-3.5 {win.maximized ? 'rotate-180' : ''}"
					/>
				</button>
			{/if}
			<button
				class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:text-destructive"
				data-window-control
				onclick={() => w.close()}
				aria-label={t('common.close')}
			>
				<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
			</button>
		{:else}
			<!-- The system frame owns closing; leave a little air before its own buttons. -->
			<div class="w-2"></div>
		{/if}
	</div>
</header>
