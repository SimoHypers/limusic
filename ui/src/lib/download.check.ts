// Self-check for the download decisions (`download.ts`): what a playlist run actually requests and
// how a remembered choice is read back. No test runner in `ui/`, node 22 runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/download.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant.
import type { SongItem } from './api.ts';
import { downloadableSongs, formatBytes, formatSpeed, isDownloadable, parseDownloadPrefs } from './download.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const song = (id: string, extra: Partial<SongItem> = {}): SongItem => ({
	video_id: id,
	title: id,
	artists: '',
	...extra
});

// --- what a playlist run requests ---------------------------------------------------------------
const playlist = [
	song('a'),
	song('b'),
	song('a', { title: 'a again' }),
	song('LOCAL:/music/c.flac'),
	song('LOCALALBUM:/music/album'),
	song('c')
];
const requested = downloadableSongs(playlist);

ok(requested.length === 3, 'three YouTube tracks out of six rows');
ok(
	requested.map((s) => s.video_id).join() === 'a,b,c',
	'first occurrence wins, playlist order is kept'
);
ok(!isDownloadable('LOCAL:/music/c.flac'), 'a local file is not downloadable');
ok(isDownloadable('dQw4w9WgXcQ'), 'a video id is downloadable');
ok(!isDownloadable(undefined) && !isDownloadable(''), 'no id means nothing to fetch');

// --- the remembered choice ----------------------------------------------------------------------
const saved = parseDownloadPrefs({
	download_format: 'flac',
	download_quality: '320',
	download_remember: 'true'
});
ok(saved.format === 'flac' && saved.quality === '320' && saved.remember, 'reads what was saved');

const fresh = parseDownloadPrefs({});
ok(
	fresh.format === 'mp3' && fresh.quality === 'best' && !fresh.remember,
	'a fresh install starts at mp3/best, not remembering'
);

const junk = parseDownloadPrefs({
	download_format: 'ogg',
	download_quality: '999',
	download_remember: 'yes'
});
ok(
	junk.format === 'mp3' && junk.quality === 'best' && !junk.remember,
	'unknown values fall back instead of reaching yt-dlp'
);

// --- the progress popover's sizes ---------------------------------------------------------------
ok(formatBytes(null) === '—' && formatBytes(0) === '—', 'nothing known shows a dash');
ok(formatBytes(512) === '512 B', 'bytes stay whole');
ok(formatBytes(1536) === '1.5 KB', 'kilobytes keep one decimal');
ok(formatBytes(12 * 1024 * 1024) === '12.0 MB', 'megabytes for a track');
ok(formatSpeed(null) === '', 'no speed until yt-dlp reports one');
ok(formatSpeed(2 * 1024 * 1024) === '2.0 MB/s', 'speed is a byte rate');

console.log('ok');
