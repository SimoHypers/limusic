export type ImportStatus = 'matched' | 'review' | 'not_found';

export interface ImportCandidate {
	song: { video_id: string; title: string; artists: string; [key: string]: any };
	score: number;
	duration_diff_secs?: number;
}

export interface ImportMatch {
	index: number;
	status: ImportStatus;
	candidates: ImportCandidate[];
}

export function applyMatchResult(
	state: { results: (ImportMatch | undefined)[]; choice: (number | null)[]; picked: boolean[] },
	result: ImportMatch
) {
	const i = result.index;
	const newResults = [...state.results];
	newResults[i] = result;
	state.results = newResults;

	const newChoice = [...state.choice];
	const newPicked = [...state.picked];

	if (result.status === 'matched') {
		newChoice[i] = result.candidates.length > 0 ? 0 : null;
		newPicked[i] = result.candidates.length > 0;
	} else if (result.status === 'review') {
		newChoice[i] = result.candidates.length > 0 ? 0 : null;
		newPicked[i] = false;
	} else {
		newChoice[i] = null;
		newPicked[i] = false;
	}
	state.choice = newChoice;
	state.picked = newPicked;
}
