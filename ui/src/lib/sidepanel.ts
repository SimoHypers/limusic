// Pure width clamping and drag calculation helpers for resizable Queue, Lyrics, and Comments
// side panels, tested by `sidepanel.check.ts` under Node.
export const SIDE_PANEL_DEFAULT = 320;
export const SIDE_PANEL_MIN = 260;
export const SIDE_PANEL_MAX = 560;
export const SIDE_PANEL_KEY = 'side_panel_width';

/** Round and clamp `px` to `[SIDE_PANEL_MIN, SIDE_PANEL_MAX]`, falling back to default if invalid. */
export function clampPanelWidth(px: number): number {
	if (!Number.isFinite(px)) return SIDE_PANEL_DEFAULT;
	return Math.round(Math.min(SIDE_PANEL_MAX, Math.max(SIDE_PANEL_MIN, px)));
}

/** Parse a persisted side-panel width from localStorage safely. */
export function parsePanelWidth(raw: string | null): number {
	if (raw === null || !raw.trim()) return SIDE_PANEL_DEFAULT;
	return clampPanelWidth(Number(raw));
}

/** Compute the new right-docked panel width when dragging its left edge from `startX` to `currentX`. */
export function dragPanelWidth(startWidth: number, startX: number, currentX: number): number {
	return clampPanelWidth(startWidth + (startX - currentX));
}
