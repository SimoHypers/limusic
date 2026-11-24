import { browser } from '$app/environment';
import {
	SIDE_PANEL_DEFAULT,
	SIDE_PANEL_KEY,
	clampPanelWidth,
	parsePanelWidth
} from './sidepanel';

function readWidth(): string | null {
	try {
		return localStorage.getItem(SIDE_PANEL_KEY);
	} catch {
		return null;
	}
}

export const sidePanel = $state({
	width: browser ? parsePanelWidth(readWidth()) : SIDE_PANEL_DEFAULT
});

export function setSidePanelWidth(px: number) {
	const next = clampPanelWidth(px);
	sidePanel.width = next;
	try {
		localStorage.setItem(SIDE_PANEL_KEY, String(next));
	} catch {}
}

export function resetSidePanelWidth() {
	setSidePanelWidth(SIDE_PANEL_DEFAULT);
}
