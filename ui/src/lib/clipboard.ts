import { writeText } from '@tauri-apps/plugin-clipboard-manager';

/**
 * Copy plain text to the system clipboard, from Rust.
 *
 * The webview cannot do this on Fedora. WebKitGTK gates JavaScript clipboard writes behind its own
 * policy, so `document.execCommand('copy')` returns false and `navigator.clipboard.writeText`
 * rejects with NotAllowedError no matter how the click is wired: every copy button in the app did
 * nothing. Writing the clipboard from the app process instead sidesteps the webview entirely, which
 * also means there is no user gesture to preserve: awaiting before this call is fine now.
 *
 * Rejects if the platform clipboard itself failed.
 */
export async function copyText(text: string): Promise<void> {
	await writeText(text);
}
