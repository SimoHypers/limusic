// Invites for Listen Together rooms and device-link pairing codes: `CODE` on the default server,
// `CODE@host` on any other. Shared by the Listen Together dialog and the Devices tab.

// An invite bundles the server + code so a guest only pastes one thing: `<code>@<host>`.
// `wss://` and the `/ws` path are implied, so the usual invite stays short and typable.
// An empty server is the built-in default, which the backend fills in, so that invite is the
// bare code: a host on the default never shares a hostname, and a guest never types one.
export function makeInvite(server: string, code: string): string {
	if (!server) return code;
	return code + '@' + server.replace(/^wss:\/\//, '').replace(/\/ws$/, '');
}
export function parseInvite(raw: string): { server: string; code: string } | null {
	const s = raw.trim();
	// Invites minted before 0.6.5: `LMSC~<base64(server|code)>`.
	if (s.startsWith('LMSC~')) {
		try {
			const [server, code] = atob(s.slice(5)).split('|');
			return { server: server ?? '', code: (code ?? '').toUpperCase() };
		} catch {
			return null;
		}
	}
	const at = s.lastIndexOf('@');
	// A bare room code: reuse whatever server we last connected to.
	if (at < 0) return { server: '', code: s.toUpperCase() };
	let server = s.slice(at + 1);
	if (!/^wss?:\/\//.test(server)) server = 'wss://' + server;
	if (!server.replace(/^wss?:\/\//, '').includes('/')) server += '/ws';
	return { server, code: s.slice(0, at).toUpperCase() };
}
