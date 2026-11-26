// Self-check for Listen Together invite codes and `limusic://` deep links (`invite.ts`).
//
//     node --experimental-strip-types ui/src/lib/invite.check.ts
import { makeInvite, makeInviteLink, parseInvite, parseInviteLink } from './invite.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

function same(
	got: { server: string; code: string } | null,
	want: { server: string; code: string } | null,
	what: string
): void {
	if (want === null) {
		ok(got === null, `${what} (expected null, got ${JSON.stringify(got)})`);
		return;
	}
	ok(
		got !== null && got.server === want.server && got.code === want.code,
		`${what} (expected ${JSON.stringify(want)}, got ${JSON.stringify(got)})`
	);
}

// --- makeInviteLink -----------------------------------------------------------------------------
ok(makeInviteLink('', 'abc123') === 'limusic://listen/ABC123', 'default server produces bare link');
ok(
	makeInviteLink('wss://sync.example.com/ws', 'ABC123') ===
		'limusic://listen/ABC123?server=sync.example.com',
	'custom wss:///ws server is shortened in query'
);
ok(
	makeInviteLink('ws://127.0.0.1:8080/ws', 'ABC123') ===
		'limusic://listen/ABC123?server=ws%3A%2F%2F127.0.0.1%3A8080',
	'non-TLS server keeps ws:// scheme'
);
ok(
	makeInviteLink('wss://sync.example.com/custom/path', 'ABC123') ===
		'limusic://listen/ABC123?server=sync.example.com%2Fcustom%2Fpath',
	'custom path is preserved'
);
ok(makeInviteLink('', '') === '', 'empty room code produces empty link');

// --- parseInviteLink ----------------------------------------------------------------------------
same(
	parseInviteLink('limusic://listen/ABC123'),
	{ server: '', code: 'ABC123' },
	'default server link'
);
same(
	parseInviteLink('  LIMUSIC://LISTEN/abc123/  '),
	{ server: '', code: 'ABC123' },
	'case-insensitive scheme/host, trimmed whitespace, trailing slash'
);
same(
	parseInviteLink('limusic://listen/ABC123?server=sync.example.com'),
	{ server: 'wss://sync.example.com/ws', code: 'ABC123' },
	'custom host expands wss:// and /ws'
);
same(
	parseInviteLink('limusic://listen/ABC123?server=ws%3A%2F%2F127.0.0.1%3A8080'),
	{ server: 'ws://127.0.0.1:8080/ws', code: 'ABC123' },
	'ws:// server expands trailing /ws'
);
same(
	parseInviteLink('limusic://listen/ABC123?server=sync.example.com%2Fcustom%2Fpath'),
	{ server: 'wss://sync.example.com/custom/path', code: 'ABC123' },
	'custom path does not get extra /ws'
);
same(
	parseInviteLink('limusic://listen/abc123@sync.example.com'),
	{ server: 'wss://sync.example.com/ws', code: 'ABC123' },
	'CODE@host path form is accepted'
);

// Round-trips
for (const server of [
	'',
	'wss://sync.example.com/ws',
	'ws://127.0.0.1:8080/ws',
	'wss://sync.example.com/custom/path'
]) {
	const link = makeInviteLink(server, 'K7QX2M');
	same(parseInviteLink(link), { server, code: 'K7QX2M' }, `round-trip for server=${server}`);
}

// Rejections
same(parseInviteLink(''), null, 'empty string');
same(parseInviteLink('ABC123'), null, 'bare code is not a link');
same(parseInviteLink('https://music.youtube.com/watch?v=dQw4'), null, 'YouTube URL is not a link');
same(parseInviteLink('limusic://other/ABC123'), null, 'unknown action host');
same(parseInviteLink('limusic://listen'), null, 'missing code');
same(parseInviteLink('limusic://listen/'), null, 'empty code with slash');
same(parseInviteLink('limusic://listen/ABC/DEF'), null, 'extra path segment');

// --- parseInvite (text input + pasted links) ----------------------------------------------------
ok(makeInvite('', 'ABC123') === 'ABC123', 'makeInvite default');
ok(makeInvite('wss://sync.example.com/ws', 'ABC123') === 'ABC123@sync.example.com', 'makeInvite custom');
same(parseInvite('abc123'), { server: '', code: 'ABC123' }, 'parseInvite bare code');
same(
	parseInvite('abc123@sync.example.com'),
	{ server: 'wss://sync.example.com/ws', code: 'ABC123' },
	'parseInvite CODE@host'
);
same(
	parseInvite('limusic://listen/abc123?server=sync.example.com'),
	{ server: 'wss://sync.example.com/ws', code: 'ABC123' },
	'parseInvite accepts pasted deep link'
);
same(parseInvite('limusic://unknown/foo'), null, 'parseInvite rejects malformed limusic:// link');

console.log('ok');
