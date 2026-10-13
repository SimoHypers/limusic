// Self-check for the URL rewrites in `thumb.ts`. Plain node, like the other checks here:
//
//     node --experimental-strip-types ui/src/lib/thumb.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app.
import { thumb } from './thumb.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

// The shape WEB_REMIX hands out for a video, /player included: 400x225 or 426x240, and the `sqp`
// crop is signed, so it can't be asked for bigger.
const video =
	'https://i.ytimg.com/vi/PtHEr7siapo/sddefault.jpg?sqp=-oaymwEWCJADEOEBIAQqCghqEJQEGHgg6AJIWg&rs=x';
ok(
	thumb(video, 720) === 'https://i.ytimg.com/vi/PtHEr7siapo/maxresdefault.jpg',
	'the big player asks for the 1280x720 video thumb (#425)'
);
ok(thumb(video, 400) === video, 'every smaller slot keeps the thumb YouTube sent');
ok(
	thumb('https://lh3.googleusercontent.com/x=w120-h120-l90-rj', 720) ===
		'https://lh3.googleusercontent.com/x=w720-h720-l90-rj',
	'album art is still resized in its URL'
);
ok(thumb('https://yt3.ggpht.com/y=s176', 400) === 'https://yt3.ggpht.com/y=s400', '=s art too');

console.log('ok');
