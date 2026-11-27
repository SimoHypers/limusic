import type { IconSvgElement } from '@hugeicons/svelte';
import { ShuffleIcon, SparklesIcon } from '@hugeicons/core-free-icons';

// Dynamic shuffle's two marks, built from HugeIcons' own path data: the free set has no shuffle
// with a sparkle. The sparkle is SparklesIcon's large star (centred at 15,9, radius 7), filled.
// SparkleIcon would be the obvious pick, but its sides barely curve in, and at 12 to 16 px it
// renders as a diamond, which is what the first version of this mark looked like.
const STAR = SparklesIcon[0][1].d as string;

/** Scale and move the star to centre (`x`, `y`) at radius `r`, in the 24-unit box. */
const star = (x: number, y: number, r: number): IconSvgElement[number] => [
	'path',
	{
		d: STAR,
		fill: 'currentColor',
		stroke: 'none',
		transform: `translate(${x} ${y}) scale(${r / 7}) translate(-15 -9)`
	}
];

/** Shuffle with a sparkle where its top-left stroke starts. The stroke is cut back by a dash (it
 *  runs from the bottom-right arrow and ends at that corner, 26.4 units long) so the two don't
 *  touch, which at 16 px would merge them into one blob. */
export const DynamicShuffleIcon: IconSvgElement = [
	['path', { ...ShuffleIcon[0][1] }],
	['path', { ...ShuffleIcon[1][1], strokeDasharray: '20.15 40' }],
	star(4.4, 4.9, 3.9)
];

/** The same sparkle on its own, filling the box: what a suggested track is marked with. */
export const SuggestedIcon: IconSvgElement = [star(12, 12, 10)];
