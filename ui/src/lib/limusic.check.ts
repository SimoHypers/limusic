// Self-check for `limusic.ts`: `node --experimental-strip-types ui/src/lib/limusic.check.ts`
import { accountErrorKey, creditNames, providerName, signInErrorKey, submitMessage } from './limusic.ts';

function ok(cond: boolean, what: string) {
	if (!cond) throw new Error(`broken: ${what}`);
}

ok(submitMessage({ kind: 'published', revision: 1 }).sent === true, 'a published lyric is sent');
ok(submitMessage({ kind: 'pending', revision: 1 }).key === 'lyrics_editor.sent_pending', 'pending says review');
const f = (code: string, detail: string | null = null) => submitMessage({ kind: 'failed', code, detail });
for (const code of ['unauthorized', 'banned', 'invalid', 'stale', 'unchanged', 'rate_limited', 'unavailable', 'whatever']) {
	const m = f(code);
	ok(!m.sent, `${code} keeps the draft`);
	ok(m.key.startsWith('lyrics_editor.'), `${code} has a message`);
	ok(m.title.startsWith('lyrics_editor.') && m.title.endsWith('_title'), `${code} has a title`);
}
ok(f('stale').stale === true && !f('stale').sent, 'stale keeps the draft and asks which version');
ok(['unauthorized', 'banned', 'invalid', 'unchanged', 'rate_limited', 'unavailable'].every((c) => !f(c).stale), 'only stale asks');
ok(signInErrorKey('rate_limited') === 'limusic.signin_rate_limited', 'link rate limit');
ok(signInErrorKey('unavailable') === 'limusic.unreachable', 'link with no connection');
ok(signInErrorKey('already_linked') === 'limusic.signin_failed', 'any other link failure');
ok(f('invalid', 'line 3 is empty').params?.detail === 'line 3 is empty', 'invalid carries the reason');
ok(f('invalid').key === 'lyrics_editor.err_invalid', 'invalid without a reason');
ok(f('unchanged').kind === 'info', 'unchanged is not an error');
ok(new Set(['stale', 'banned', 'rate_limited', 'unavailable'].map((c) => f(c).key)).size === 4, 'distinct words');
ok(accountErrorKey('taken') === 'limusic.name_taken' && accountErrorKey('?') === 'limusic.unreachable', 'account codes');
ok(providerName('github') === 'GitHub' && providerName('gitlab') === 'Gitlab', 'provider names');
ok(providerName('google') === 'Google', 'an unknown id is capitalised, Google is no longer a provider');
ok(creditNames(['carla'], 'en') === 'carla', 'one contributor');
ok(creditNames(['carla', 'moss_fern'], 'en') === 'carla and moss_fern', 'two contributors');
ok(creditNames(['a', 'b', 'c'], 'en') === 'a, b, and c', 'the locale lists them');
ok(creditNames(['a', 'b'], 'de') === 'a und b', 'in its own words');
console.log('ok');
