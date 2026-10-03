// Self-check for playlist importer state updates (`importer-helpers.ts`).
// Run with: node --experimental-strip-types ui/src/lib/importer.check.ts

import { applyMatchResult, type ImportMatch } from './importer-helpers.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const state = {
	results: [] as (ImportMatch | undefined)[],
	choice: [] as (number | null)[],
	picked: [] as boolean[]
};

const matchRes: ImportMatch = {
	index: 0,
	status: 'matched',
	candidates: [
		{
			song: { video_id: 'v1', title: 'Song 1', artists: 'Artist 1' },
			score: 0.95,
			duration_diff_secs: null
		}
	]
};

applyMatchResult(state, matchRes);
ok(state.results[0]?.status === 'matched', 'result status is matched');
ok(state.choice[0] === 0, 'choice is set to 0 for matched');
ok(state.picked[0] === true, 'picked is true for matched');

const reviewRes: ImportMatch = {
	index: 1,
	status: 'review',
	candidates: [
		{
			song: { video_id: 'v2', title: 'Song 2', artists: 'Artist 2' },
			score: 0.6,
			duration_diff_secs: null
		}
	]
};

applyMatchResult(state, reviewRes);
ok(state.choice[1] === 0, 'choice is 0 for review');
ok(state.picked[1] === false, 'picked is false for review');

const notFoundRes: ImportMatch = {
	index: 2,
	status: 'not_found',
	candidates: []
};

applyMatchResult(state, notFoundRes);
ok(state.choice[2] === null, 'choice is null for not_found');
ok(state.picked[2] === false, 'picked is false for not_found');

console.log('ok');
