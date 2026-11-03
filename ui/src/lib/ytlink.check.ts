// Self-check for the link parser in `ytlink.ts`. There is no test runner in `ui/` and this doesn't
// warrant adding one — node 22 runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/ytlink.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app.
import { parseYtLink } from './ytlink.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const target = (url: string) => {
	const t = parseYtLink(url);
	return t ? `${t.kind}:${t.id}` : null;
};

// --- what the share sheets actually hand out ----------------------------------------------------
ok(
	target('https://music.youtube.com/playlist?list=PLabc123') === 'playlist:VLPLabc123',
	'a playlist link gets the VL prefix browse wants'
);
ok(
	target('https://music.youtube.com/playlist?list=VLPLabc123') === 'playlist:VLPLabc123',
	'a list id that already carries VL is not doubled'
);
ok(
	target('https://music.youtube.com/playlist?list=OLAK5uy_abc') === 'playlist:VLOLAK5uy_abc',
	"an album's audio playlist opens as a playlist"
);
ok(
	target('https://music.youtube.com/browse/MPREb_abc') === 'album:MPREb_abc',
	'YTM album pages are browse ids'
);
ok(
	target('https://music.youtube.com/channel/UCabc') === 'artist:UCabc',
	'a channel link is the artist'
);
ok(target('https://music.youtube.com/browse/UCabc') === 'artist:UCabc', 'so is /browse of one');
ok(target('https://music.youtube.com/watch?v=dQw4') === 'song:dQw4', 'a watch link is the song');
ok(
	target('https://music.youtube.com/watch?v=dQw4&list=PLabc') === 'song:dQw4',
	'a track shared out of a playlist opens on the track'
);
ok(target('https://youtu.be/dQw4?si=xyz') === 'song:dQw4', 'youtu.be short links, tracking and all');
ok(target('https://www.youtube.com/watch?v=dQw4') === 'song:dQw4', 'the video site counts too');
ok(target('music.youtube.com/playlist?list=PLabc') === 'playlist:VLPLabc', 'a pasted bare host');
ok(target('  https://youtu.be/dQw4  ') === 'song:dQw4', 'surrounding whitespace is trimmed');

// --- start time (`t=`) ---------------------------------------------------------------------------
const start = (url: string) => parseYtLink(url)?.start;
ok(start('https://music.youtube.com/watch?v=dQw4&t=83') === 83, 't=83');
ok(start('https://music.youtube.com/watch?v=dQw4&t=1m23s') === 83, 't=1m23s');
ok(start('https://www.youtube.com/watch?v=dQw4&start=90') === 90, 'start= is the embed spelling');
ok(start('https://youtu.be/dQw4?t=90') === 90, 'youtu.be short link');
ok(start('https://youtu.be/dQw4?si=xyz&t=90') === 90, 't= after tracking params');
ok(start('https://music.youtube.com/watch?v=dQw4&list=PLabc&t=12') === 12, 'inside a playlist');
ok(start('https://music.youtube.com/watch?v=dQw4') === undefined, 'no t, no start');
ok(start('https://music.youtube.com/watch?v=dQw4&t=abc') === undefined, 'a bad t is ignored');
ok(start('https://music.youtube.com/watch?v=dQw4&t=0') === undefined, 't=0 is just the start');
ok(target('https://music.youtube.com/watch?v=dQw4&t=83') === 'song:dQw4', 't= does not change the id');
ok(start('https://music.youtube.com/playlist?list=PLabc&t=83') === undefined, 'playlists have no start');

// --- and what must not open anything ------------------------------------------------------------
ok(parseYtLink('') === null, 'an empty box is not an error to report');
ok(parseYtLink('not a url at all') === null, 'plain text');
ok(parseYtLink('https://example.com/playlist?list=PLabc') === null, 'a domain that is not YouTube');
ok(parseYtLink('https://youtube.com.evil.test/watch?v=x') === null, 'a lookalike host');
ok(parseYtLink('https://music.youtube.com/browse/FEmusic_home') === null, 'a browse id with no page of ours');
ok(parseYtLink('https://music.youtube.com/') === null, 'the bare site');

console.log('ok');
