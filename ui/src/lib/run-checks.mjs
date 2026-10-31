// Runs every *.check.ts / *.check.mjs self-check here, one child process each, failing if any fails.
// The checks are plain Node scripts (see the header of queue.check.ts): they throw on a
// broken invariant and print "ok" otherwise. They existed for months with no caller, which is how
// a self-check quietly stops being one.
import { readdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const files = readdirSync(here).filter((f) => /\.check\.(ts|mjs)$/.test(f)).sort();
if (files.length === 0) {
	console.error('no self-check files found, which is itself a failure');
	process.exit(1);
}

let failed = 0;
for (const f of files) {
	try {
		const args = f.endsWith('.ts') ? ['--experimental-strip-types', join(here, f)] : [join(here, f)];
		execFileSync(process.execPath, args, {
			stdio: 'inherit'
		});
		console.log(`PASS ${f}`);
	} catch {
		console.error(`FAIL ${f}`);
		failed++;
	}
}
console.log(`${files.length - failed}/${files.length} self-checks passed`);
process.exit(failed === 0 ? 0 : 1);
