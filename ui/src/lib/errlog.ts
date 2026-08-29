// Webview errors into `limusic.log`, so the diagnostics blob a user hands over covers the half of
// the app that runs in JavaScript. Before this, a blank screen or a rejected `invoke` left nothing
// in the log at all and the report was a screenshot of nothing happening.
import { invoke } from '@tauri-apps/api/core';

let last = '';
let lastAt = 0;

/** Send one line to the Rust log. Never throws: logging a failure must not cause one. */
export function logUi(level: 'info' | 'warn' | 'error', message: string) {
	const now = Date.now();
	// An error thrown inside an `$effect` re-fires every frame; the log file has no size cap.
	if (message === last && now - lastAt < 5000) return;
	last = message;
	lastAt = now;
	invoke('log_ui', { level, message }).catch(() => {});
}

/** Install the global handlers. Call once per window, as early as possible. */
export function initErrorLog() {
	// The engine string is the first thing asked about on a rendering bug, and WebKitGTK's release
	// is not in it, so it goes in the log where the report picks it up for free.
	logUi('info', `webview ${navigator.userAgent}`);
	window.addEventListener('error', (e) =>
		logUi('error', `${e.message} (${e.filename}:${e.lineno}:${e.colno})`)
	);
	window.addEventListener('unhandledrejection', (e) =>
		logUi('error', `unhandled rejection: ${describe(e.reason)}`)
	);
}

function describe(reason: unknown): string {
	if (reason instanceof Error) return `${reason.message}\n${reason.stack ?? ''}`;
	return String(reason);
}
