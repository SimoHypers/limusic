import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
	return twMerge(clsx(inputs));
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type WithoutChild<T> = T extends { child?: any } ? Omit<T, "child"> : T;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type WithoutChildren<T> = T extends { children?: any } ? Omit<T, "children"> : T;
export type WithoutChildrenOrChild<T> = WithoutChildren<WithoutChild<T>>;
export type WithElementRef<T, U extends HTMLElement = HTMLElement> = T & { ref?: U | null };

/** The titlebar sits above dialog backdrops (#427). A press on its drag region or a window button
 *  moves or resizes the window, it isn't a click on the page behind, so a dialog's
 *  `onInteractOutside` should leave the dialog open. The bar's app buttons still dismiss it. */
export function onWindowChrome(e: Event) {
	const el = e.target as Element;
	return !!(el.matches?.("[data-tauri-drag-region]") || el.closest?.("[data-window-control]"));
}
