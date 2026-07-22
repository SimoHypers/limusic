// Anchor a `fixed` dropdown at its trigger, flipping above it when the viewport bottom is too
// close for the menu to fit. `y` pairs with `top` normally, `bottom` when flipped.
// ponytail: estHeight over measuring — menus here are small and a few px of early flip is fine.
export function anchorMenu(trigger: HTMLElement, estHeight = 280) {
	const r = trigger.getBoundingClientRect();
	const openUp = r.bottom + estHeight > window.innerHeight;
	return {
		right: window.innerWidth - r.right,
		y: openUp ? window.innerHeight - r.top + 4 : r.bottom + 4,
		openUp
	};
}
