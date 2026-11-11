// Self-check for `limusic.ts`: `node --experimental-strip-types ui/src/lib/limusic.check.ts`
import { accountErrorKey, providerName, submitMessage } from './limusic.ts';

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
}
ok(f('unauthorized').needsAccount === true, 'signed out points at the account card');
ok(f('invalid', 'line 3 is empty').params?.detail === 'line 3 is empty', 'invalid carries the reason');
ok(f('invalid').key === 'lyrics_editor.err_invalid', 'invalid without a reason');
ok(f('unchanged').kind === 'info', 'unchanged is not an error');
ok(new Set(['stale', 'banned', 'rate_limited', 'unavailable'].map((c) => f(c).key)).size === 4, 'distinct words');
ok(accountErrorKey('taken') === 'limusic.name_taken' && accountErrorKey('?') === 'limusic.unreachable', 'account codes');
ok(providerName('github') === 'GitHub' && providerName('gitlab') === 'Gitlab', 'provider names');
console.log('ok');
