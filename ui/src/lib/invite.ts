// Invites for Listen Together rooms and device-link pairing codes: `CODE` on the default server,
// `CODE@host` on any other, and `limusic://listen/CODE` links for one-click joining. Shared by
// the Listen Together dialog, the Open Link handler, and the Devices tab.

function shortenServer(server: string): string {
	return server.replace(/^wss:\/\//, '').replace(/\/ws$/, '');
}

function expandServer(raw: string): string {
	let server = raw.trim();
	if (!server) return '';
	if (!/^wss?:\/\//.test(server)) server = 'wss://' + server;
	if (!server.replace(/^wss?:\/\//, '').includes('/')) server += '/ws';
	return server;
}

// An invite bundles the server + code so a guest only pastes one thing: `<code>@<host>`.
// `wss://` and the `/ws` path are implied, so the usual invite stays short and typable.
// An empty server is the built-in default, which the backend fills in, so that invite is the
// bare code: a host on the default never shares a hostname, and a guest never types one.
export function makeInvite(server: string, code: string): string {
	if (!server) return code;
	return code + '@' + shortenServer(server);
}

/** Shareable `limusic://` link that opens Limusic directly into a Listen Together room. */
export function makeInviteLink(server: string, code: string): string {
	const cleanCode = code.trim().toUpperCase();
	if (!cleanCode) return '';
	const base = `limusic://listen/${encodeURIComponent(cleanCode)}`;
	if (!server) return base;
	return `${base}?server=${encodeURIComponent(shortenServer(server))}`;
}

/** Parse a `limusic://listen/...` deep link, or `null` for anything else. */
export function parseInviteLink(raw: string): { server: string; code: string } | null {
	const s = raw.trim();
	if (!/^limusic:\/\//i.test(s)) return null;
	let u: URL;
	try {
		u = new URL(s);
	} catch {
		return null;
	}
	if (u.hostname.toLowerCase() !== 'listen') return null;
	let seg: string;
	try {
		seg = decodeURIComponent(u.pathname.replace(/^\/+|\/+$/g, ''));
	} catch {
		return null;
	}
	if (!seg || seg.includes('/')) return null;
	const serverParam = u.searchParams.get('server')?.trim() ?? '';
	if (serverParam) {
		return { server: expandServer(serverParam), code: seg.toUpperCase() };
	}
	const at = seg.lastIndexOf('@');
	if (at >= 0) {
		const code = seg.slice(0, at).trim().toUpperCase();
		const host = seg.slice(at + 1).trim();
		if (!code) return null;
		return { server: host ? expandServer(host) : '', code };
	}
	return { server: '', code: seg.toUpperCase() };
}

export function parseInvite(raw: string): { server: string; code: string } | null {
	const s = raw.trim();
	if (/^limusic:\/\//i.test(s)) return parseInviteLink(s);
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
	return { server: expandServer(s.slice(at + 1)), code: s.slice(0, at).toUpperCase() };
}
